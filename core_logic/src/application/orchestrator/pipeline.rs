impl Orchestrator {
    fn parse_tool_error_code(result_json: &str) -> Option<String> {
        let value: Value = serde_json::from_str(result_json).ok()?;
        let object = value.as_object()?;

        let status_failed = object
            .get("status")
            .and_then(Value::as_str)
            .map(|status| matches!(status.to_ascii_lowercase().as_str(), "error" | "failed"))
            .unwrap_or(false);
        let ok_false = object.get("ok").and_then(Value::as_bool) == Some(false);
        let success_false = object.get("success").and_then(Value::as_bool) == Some(false);
        if !(status_failed || ok_false || success_false || object.contains_key("error")) {
            return None;
        }

        object
            .get("errorCode")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| {
                object
                    .get("error")
                    .and_then(Value::as_object)
                    .and_then(|err| err.get("code"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .or_else(|| {
                object
                    .get("code")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .or_else(|| Some("TOOL_EXECUTION_FAILED".to_string()))
    }

    fn collect_observation_names(result_json: &str, field: &str) -> Vec<String> {
        let value: Value = match serde_json::from_str(result_json) {
            Ok(value) => value,
            Err(_) => return Vec::new(),
        };
        value
            .get(field)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| {
                item.get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(ToString::to_string)
            })
            .collect()
    }

    fn observation_field_has_items(result_json: &str, field: &str) -> bool {
        let value: Value = match serde_json::from_str(result_json) {
            Ok(value) => value,
            Err(_) => return false,
        };
        value
            .get(field)
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
    }

    fn observation_binding_satisfied(role: &AgentRole, result: &MixedResult, result_json: &str) -> bool {
        match role {
            AgentRole::TaskAgent => {
                let names = Self::collect_observation_names(result_json, "tasks");
                if names.is_empty() {
                    return true;
                }
                let result_names = result
                    .tasks
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .map(|task| task.name.as_str())
                    .chain(
                        result
                            .retrieved_tasks
                            .as_ref()
                            .into_iter()
                            .flatten()
                            .map(|task| task.name.as_str()),
                    )
                    .collect::<Vec<_>>();
                names.iter().any(|name| {
                    result_names.iter().any(|result_name| *result_name == name)
                        || result
                            .chat_response
                            .as_deref()
                            .unwrap_or("")
                            .contains(name)
                })
            }
            AgentRole::HabitAgent => {
                let names = Self::collect_observation_names(result_json, "habits");
                if names.is_empty() {
                    return true;
                }
                let result_names = result
                    .habits
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .map(|habit| habit.name.as_str())
                    .chain(
                        result
                            .retrieved_habits
                            .as_ref()
                            .into_iter()
                            .flatten()
                            .map(|habit| habit.name.as_str()),
                    )
                    .collect::<Vec<_>>();
                names.iter().any(|name| {
                    result_names.iter().any(|result_name| *result_name == name)
                        || result
                            .chat_response
                            .as_deref()
                            .unwrap_or("")
                            .contains(name)
                })
            }
            _ => true,
        }
    }

    fn apply_retry_convergence_policy(
        context: &PendingToolContext,
        tool_error_code: Option<&str>,
        result: &mut MixedResult,
    ) -> Result<(), AIError> {
        if tool_error_code.is_none() {
            return Ok(());
        }

        if context.retry_count >= 1 {
            return Err(AIError::InvalidToolCall(format!(
                "工具 {} 已重试过一次（errorCode={}），禁止继续软兜底",
                context.tool_name,
                tool_error_code.unwrap_or("UNKNOWN")
            )));
        }

        let Some(tool_calls) = result.tool_calls.as_mut() else {
            return Err(AIError::InvalidToolCall(format!(
                "工具 {} 执行失败（errorCode={}）后必须继续给出同工具重试调用",
                context.tool_name,
                tool_error_code.unwrap_or("UNKNOWN")
            )));
        };

        tool_calls.retain(|call| call.tool == context.tool_name);
        for tool_call in tool_calls.iter_mut() {
            tool_call.execution_mode = Some(CapabilityExecutionMode::AskUser);
            if let Some(args) = tool_call.args.as_object_mut() {
                let meta = args.entry("_meta".to_string()).or_insert_with(|| serde_json::json!({}));
                if let Some(meta_obj) = meta.as_object_mut() {
                    meta_obj.insert(
                        "retryCount".to_string(),
                        serde_json::json!(u64::from(context.retry_count + 1)),
                    );
                    meta_obj.insert(
                        "retryFromErrorCode".to_string(),
                        serde_json::json!(tool_error_code.unwrap_or("UNKNOWN")),
                    );
                }
            }
            let guard_reason = format!(
                "[retry_guard allowedTool={} errorCode={}]",
                context.tool_name,
                tool_error_code.unwrap_or("UNKNOWN")
            );
            tool_call.reason = Some(match &tool_call.reason {
                Some(existing) if !existing.trim().is_empty() => {
                    format!("{} {}", existing, guard_reason)
                }
                _ => guard_reason,
            });
        }

        if tool_calls.is_empty() {
            return Err(AIError::InvalidToolCall(format!(
                "工具执行失败（errorCode={}）后禁止切换工具，当前必须继续调用 {}",
                tool_error_code.unwrap_or("UNKNOWN"),
                context.tool_name
            )));
        }
        Ok(())
    }

    pub async fn process_message_core(&self, text: String) -> Result<MixedResult, AIError> {
        let _ = self
            .memory_store
            .append_conversation_turn("user".to_string(), text.clone())
            .await;

        // 1. 第一阶段：路由
        let route_info = self.route(&text).await?;
        let mut routed_agents = route_info.routed_agents.clone().unwrap_or_default();
        let has_domain_specialist = routed_agents
            .iter()
            .any(|agent| matches!(agent, AgentRole::TaskAgent | AgentRole::HabitAgent));
        if has_domain_specialist {
            routed_agents.retain(|agent| !matches!(agent, AgentRole::ChatAgent));
        }

        if routed_agents.is_empty() {
            return Ok(MixedResult {
                chat_response: Some("抱歉，我没太明白您的意思。".to_string()),
                ..Default::default()
            });
        }

        for agent in &routed_agents {
            self.emit_thinking(
                agent.as_label(),
                "routing",
                Some("已被路由选中，准备开始工作".to_string()),
            )
            .await;
        }

        // 2. 第二阶段：并行调度专家 (Section 4)
        let mut futures = Vec::new();
        for agent in routed_agents {
            futures.push(self.call_worker(agent, &text));
        }

        let worker_results = join_all(futures).await;

        // 3. 第三阶段：合并结果
        let mut final_result = MixedResult {
            primary_intent: route_info.primary_intent,
            routed_agents: route_info.routed_agents, // 保留路由信息
            ..Default::default()
        };

        for res in worker_results {
            let worker_res = res?;
            let worker_res: MixedResult = worker_res.into();
            if let Some(tasks) = worker_res.tasks {
                let mut current_tasks = final_result.tasks.unwrap_or_default();
                current_tasks.extend(tasks);
                final_result.tasks = Some(current_tasks);
            }
            if let Some(habits) = worker_res.habits {
                let mut current_habits = final_result.habits.unwrap_or_default();
                current_habits.extend(habits);
                final_result.habits = Some(current_habits);
            }
            if let Some(retrieved_tasks) = worker_res.retrieved_tasks {
                let mut current_retrieved_tasks = final_result.retrieved_tasks.unwrap_or_default();
                current_retrieved_tasks.extend(retrieved_tasks);
                final_result.retrieved_tasks = Some(current_retrieved_tasks);
            }
            if let Some(retrieved_habits) = worker_res.retrieved_habits {
                let mut current_retrieved_habits =
                    final_result.retrieved_habits.unwrap_or_default();
                current_retrieved_habits.extend(retrieved_habits);
                final_result.retrieved_habits = Some(current_retrieved_habits);
            }
            if let Some(tool_calls) = worker_res.tool_calls {
                let mut current_tool_calls = final_result.tool_calls.unwrap_or_default();
                current_tool_calls.extend(tool_calls);
                final_result.tool_calls = Some(current_tool_calls);
            }
            if let Some(new_memories) = worker_res.new_memories {
                let mut current_memories = final_result.new_memories.unwrap_or_default();
                current_memories.extend(new_memories);
                final_result.new_memories = Some(current_memories);
            }
            if let Some(chat_response) = worker_res.chat_response {
                match &mut final_result.chat_response {
                    Some(existing) if !existing.is_empty() => {
                        if !chat_response.is_empty() {
                            existing.push_str("\n");
                            existing.push_str(&chat_response);
                        }
                    }
                    _ => {
                        final_result.chat_response = Some(chat_response);
                    }
                }
            }
            if let Some(session_summary) = worker_res.session_summary {
                final_result.session_summary = Some(session_summary);
            }
            if let Some(user_profile) = worker_res.user_profile {
                final_result.user_profile = Some(user_profile);
            }
        }

        if final_result
            .tasks
            .as_ref()
            .map(|tasks| tasks.is_empty())
            .unwrap_or(true)
            && final_result
                .habits
                .as_ref()
                .map(|habits| habits.is_empty())
                .unwrap_or(true)
            && final_result
                .chat_response
                .as_deref()
                .map(|chat| !chat.trim().is_empty())
                .unwrap_or(false)
        {
            final_result.primary_intent = Some("Chat".to_string());
        }

        self.append_assistant_turn(&final_result).await;
        Ok(final_result)
    }

    pub async fn submit_tool_result_core(
        &self,
        tool_call_id: String,
        result_json: String,
    ) -> Result<(String, MixedResult), AIError> {
        let context = {
            let mut pending = self.pending_tool_calls.lock().await;
            pending.remove(&tool_call_id)
        }
        .ok_or_else(|| {
            AIError::InvalidToolCall(format!("未找到待回灌的工具调用 {}", tool_call_id))
        })?;

        let original_text = context.original_text.clone();
        let tool_error_code = Self::parse_tool_error_code(&result_json);
        let mut result = self
            .continue_with_tool_observation(
                context.role.clone(),
                original_text.clone(),
                result_json.clone(),
                Some(context.tool_name.as_str()),
                tool_error_code.as_ref().map(|_| context.tool_name.as_str()),
                tool_error_code.as_deref(),
                false,
                false,
            )
            .await?;
        Self::apply_retry_convergence_policy(
            &context,
            tool_error_code.as_deref(),
            &mut result,
        )?;
        if !Self::observation_binding_satisfied(&context.role, &result, &result_json) {
            let mut rebound = self
                .continue_with_tool_observation(
                    context.role.clone(),
                    original_text.clone(),
                    result_json.clone(),
                    Some(context.tool_name.as_str()),
                    tool_error_code.as_ref().map(|_| context.tool_name.as_str()),
                    tool_error_code.as_deref(),
                    true,
                    false,
                )
                .await?;
            Self::apply_retry_convergence_policy(
                &context,
                tool_error_code.as_deref(),
                &mut rebound,
            )?;
            result = rebound;
        }

        let needs_followup_write_call = matches!(context.role, AgentRole::TaskAgent)
            && context.tool_name == "read_tasks"
            && !Self::is_query_request(&context.role, &original_text)
            && !Self::observation_field_has_items(&result_json, "tasks")
            && result
                .tool_calls
                .as_ref()
                .map(|calls| calls.is_empty())
                .unwrap_or(true);
        if needs_followup_write_call {
            result = self
                .continue_with_tool_observation(
                    context.role.clone(),
                    original_text.clone(),
                    result_json.clone(),
                    Some(context.tool_name.as_str()),
                    tool_error_code.as_ref().map(|_| context.tool_name.as_str()),
                    tool_error_code.as_deref(),
                    false,
                    true,
                )
                .await?;
            Self::apply_retry_convergence_policy(
                &context,
                tool_error_code.as_deref(),
                &mut result,
            )?;
        }
        if result
            .tasks
            .as_ref()
            .map(|tasks| tasks.is_empty())
            .unwrap_or(true)
            && result
                .habits
                .as_ref()
                .map(|habits| habits.is_empty())
                .unwrap_or(true)
            && result
                .chat_response
                .as_deref()
                .map(|chat| !chat.trim().is_empty())
                .unwrap_or(false)
        {
            result.primary_intent = Some("Chat".to_string());
        }
        self.append_assistant_turn(&result).await;
        Ok((original_text, result))
    }

    pub async fn synthesize_and_commit_memory(
        &self,
        original_text: String,
        final_result: MixedResult,
    ) -> Result<MemoryCommitResult, AIError> {
        let _final_result = self
            .finalize_result_with_memory(&original_text, final_result)
            .await?;

        let payload = self.memory_store.drain_pending_sync().await?;
        let memory_sync_json = if payload.operations.is_empty() {
            None
        } else {
            serde_json::to_string(&payload).ok()
        };

        let mut commit_result = MemoryCommitResult {
            new_revision: payload.next_revision,
            memory_sync_json,
            ..Default::default()
        };

        for operation in payload.operations {
            match operation {
                crate::domain::models::MemorySyncOperation::UpsertFragment { fragment } => {
                    commit_result.added_memories.push(fragment.content);
                }
                crate::domain::models::MemorySyncOperation::ReplaceUserProfile { .. } => {
                    commit_result.profile_updated = true;
                }
                crate::domain::models::MemorySyncOperation::DeleteFragment { .. } => {}
            }
        }

        Ok(commit_result)
    }
}

#[async_trait]
impl AgentOrchestrator for Orchestrator {
    async fn process_message(&self, text: String) -> Result<MixedResult, AIError> {
        let result = self.process_message_core(text.clone()).await?;
        self.finalize_result_with_memory(&text, result).await
    }
}

#[async_trait]
impl ToolExecutor for Orchestrator {
    async fn submit_tool_result(
        &self,
        tool_call_id: String,
        result_json: String,
    ) -> Result<MixedResult, AIError> {
        let (original_text, result) = self
            .submit_tool_result_core(tool_call_id, result_json)
            .await?;
        self.finalize_result_with_memory(&original_text, result)
            .await
    }
}

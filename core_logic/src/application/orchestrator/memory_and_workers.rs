impl Orchestrator {
    async fn memory_context_message(&self) -> Option<ChatMessage> {
        match self.memory_store.get_long_term_context().await {
            Ok(context) if !context.trim().is_empty() => Some(ChatMessage {
                role: "system".to_string(),
                content: format!("以下是用户的长期记忆上下文，请在推理时参考：\n{}", context),
            }),
            _ => None,
        }
    }

    async fn conversation_context_message(&self) -> Option<ChatMessage> {
        match self
            .memory_store
            .get_recent_conversation_context(MAX_CONVERSATION_CONTEXT_TURNS)
            .await
        {
            Ok(context) if !context.trim().is_empty() => Some(ChatMessage {
                role: "system".to_string(),
                content: format!("以下是最近几轮本地对话历史，请结合上下文回答：\n{}", context),
            }),
            _ => None,
        }
    }

    async fn append_assistant_turn(&self, result: &MixedResult) {
        let content = result
            .chat_response
            .clone()
            .or_else(|| result.session_summary.clone())
            .or_else(|| {
                let task_count = result.tasks.as_ref().map(|items| items.len()).unwrap_or(0);
                let habit_count = result.habits.as_ref().map(|items| items.len()).unwrap_or(0);
                let tool_count = result
                    .tool_calls
                    .as_ref()
                    .map(|items| items.len())
                    .unwrap_or(0);
                if task_count > 0 || habit_count > 0 || tool_count > 0 {
                    Some(format!(
                        "本轮处理结果：tasks={}, habits={}, toolCalls={}",
                        task_count, habit_count, tool_count
                    ))
                } else {
                    None
                }
            });

        if let Some(content) = content {
            let _ = self
                .memory_store
                .append_conversation_turn("assistant".to_string(), content)
                .await;
        }
    }

    async fn persist_memory_updates(&self, result: &MixedResult) -> Result<(), AIError> {
        if let Some(profile) = &result.user_profile {
            self.memory_store.save_user_profile(profile.clone()).await?;
        }

        if let Some(memories) = &result.new_memories {
            for memory in memories {
                self.memory_store
                    .save_memory(memory.clone(), "assistant_inferred".to_string())
                    .await?;
            }
        }

        Ok(())
    }

    async fn synthesize_memory(
        &self,
        original_text: &str,
        final_result: &MixedResult,
    ) -> Result<MixedResult, AIError> {
        self.emit_thinking(
            "Supervisor",
            "memory",
            Some("正在整理并写入记忆".to_string()),
        )
        .await;

        let system_prompt = PromptBuilder::build_memory_synthesizer_prompt(&self.preferred_lang);
        let final_result_json =
            serde_json::to_string_pretty(final_result).map_err(|_| AIError::ParsingFailed)?;

        let synthesis_prompt = format!(
            "原始用户输入：{}\n\n本轮已经确认的最终结构化结果：{}\n\n请只输出记忆相关 JSON。",
            original_text, final_result_json
        );

        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
        }];
        if let Some(memory_message) = self.memory_context_message().await {
            messages.push(memory_message);
        }
        if let Some(conversation_message) = self.conversation_context_message().await {
            messages.push(conversation_message);
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: synthesis_prompt,
        });

        let request = self.get_chat_request(messages);

        let response = self.llm_gateway.chat(request).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        logger::debug(&format!("Memory Synthesis Response: {}", content));

        let json_str = Self::extract_json(&content)?;
        let memory_result: MixedResult =
            serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;
        Ok(memory_result)
    }

    fn apply_memory_result(final_result: &mut MixedResult, memory_result: MixedResult) {
        final_result.new_memories = memory_result.new_memories;
        final_result.user_profile = memory_result.user_profile;
        final_result.session_summary = memory_result
            .session_summary
            .or(final_result.session_summary.clone());
    }

    async fn finalize_result_with_memory(
        &self,
        original_text: &str,
        mut final_result: MixedResult,
    ) -> Result<MixedResult, AIError> {
        let memory_result = self.synthesize_memory(original_text, &final_result).await?;
        Self::apply_memory_result(&mut final_result, memory_result);
        self.persist_memory_updates(&final_result).await?;
        Ok(final_result)
    }

    pub async fn register_pending_tool_call(
        &self,
        tool_call_id: String,
        role: AgentRole,
        original_text: String,
        tool_name: String,
        retry_count: u8,
    ) {
        let mut pending = self.pending_tool_calls.lock().await;
        pending.insert(
            tool_call_id,
            PendingToolContext {
                role,
                original_text,
                tool_name,
                retry_count,
            },
        );
    }

    async fn continue_with_tool_observation(
        &self,
        role: AgentRole,
        original_text: String,
        result_json: String,
        executed_tool_name: Option<&str>,
        retry_tool_name: Option<&str>,
        tool_error_code: Option<&str>,
        enforce_observation_binding: bool,
        enforce_followup_write_call: bool,
    ) -> Result<MixedResult, AIError> {
        self.emit_thinking(
            role.as_label(),
            "working",
            Some("收到工具结果，正在继续整理答案".to_string()),
        )
        .await;

        let tools = self
            .skill_registry
            .for_role_and_platform(&role, &self.platform);
        let system_prompt = match role {
            AgentRole::TaskAgent => {
                PromptBuilder::build_task_agent_prompt(&self.preferred_lang, &tools)
            }
            AgentRole::HabitAgent => {
                PromptBuilder::build_habit_agent_prompt(&self.preferred_lang, &tools)
            }
            AgentRole::ChatAgent => PromptBuilder::build_chat_agent_prompt(&self.preferred_lang),
            _ => "你是 Deadliner AI 助手。".to_string(),
        };

        let retry_guard = if let Some(tool_name) = retry_tool_name {
            format!(
                r#"
补充约束（工具失败重试收敛）：
- 上一轮工具执行失败，errorCode: {}
- 如果你认为需要再次调用工具，只允许继续调用同一个工具：{}。
- 你只能修正参数，禁止切换工具名称。"#,
                tool_error_code.unwrap_or("UNKNOWN"),
                tool_name
            )
        } else {
            String::new()
        };

        let binding_guard = if enforce_observation_binding {
            r#"
补充约束（Observation 绑定）：
- 你上一版答案没有充分吸收 observation 关键字段。
- 这一次必须显式吸收 observation 中的关键信息（例如任务名称、数量、时间等）。
- 若仍无法吸收，请给出 chatResponse 解释原因，且禁止输出 toolCalls。"#.to_string()
        } else {
            String::new()
        };

        let followup_write_guard = if enforce_followup_write_call {
            format!(
                r#"
补充约束（查重后继续执行）：
- 你上一轮先调用了 {} 做查重/读取上下文，observation 显示当前没有可匹配的重复任务。
- 原始用户请求是新增或改期，不是查询，所以这一轮不能只给 chatResponse。
- 你必须输出下一步写操作 toolCall：
  - 新增任务：create_task
  - 修改截止时间：update_deadline
  - 新增习惯：create_habit
- 如果参数还不完整，也必须输出上述写操作 toolCall，并将 executionMode 设为 "ASK_USER"。"#,
                executed_tool_name.unwrap_or("read_tasks")
            )
        } else {
            String::new()
        };

        let observation_prompt = format!(
            r#"你现在处于工具调用后的第二阶段整理。

原始用户输入：
{}

工具执行结果 JSON：
{}

请严格遵守以下规则：
1. 你必须基于上面的工具结果进行最终整理，不能忽略 observation。
2. 如果工具结果里已经出现可直接使用的 tasks/habits，就应该优先吸收并输出，而不是返回空数组。
3. 只有当工具结果明确表明没有相关数据，且用户输入也不足以生成结果时，才能输出空数组。
4. 这一轮默认应该给最终答案；若信息已经足够，禁止再次输出 toolCalls。
5. 如果原始用户输入是在查看/列出/总结已有内容，那么这是查询请求，不是新增请求。此时应优先输出 chatResponse，总结 observation；tasks/habits 应为空数组，除非用户明确要求新增或修改。
6. 只输出纯 JSON。{}{}{}"#,
            original_text, result_json, retry_guard, binding_guard, followup_write_guard
        );

        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
        }];
        if let Some(memory_message) = self.memory_context_message().await {
            messages.push(memory_message);
        }
        if let Some(conversation_message) = self.conversation_context_message().await {
            messages.push(conversation_message);
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: observation_prompt,
        });

        let request = self.get_chat_request(messages);

        let response = self.llm_gateway.chat(request).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        logger::debug(&format!("Observation Response: {}", content));

        let json_str = Self::extract_json(&content)?;
        let mut worker_result: MixedResult =
            serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;
        self.normalize_and_validate_tool_calls_guarded(&role, &original_text, &mut worker_result)?;
        Self::apply_observation_fallback(&role, &result_json, &mut worker_result);
        Self::normalize_query_result(&role, &original_text, &mut worker_result);
        if let Some(tool_calls) = &mut worker_result.tool_calls {
            for tool_call in tool_calls.iter_mut() {
                tool_call.agent = Some(role.clone());
            }
        }
        Ok(worker_result)
    }

    /// 第一阶段：Supervisor 路由
    async fn route(&self, text: &str) -> Result<PlannerResult, AIError> {
        self.emit_thinking(
            AgentRole::Supervisor.as_label(),
            "routing",
            Some("正在判断应该由哪些专家处理".to_string()),
        )
        .await;

        let system_prompt = PromptBuilder::build_supervisor_prompt(&self.preferred_lang);
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
        }];
        if let Some(memory_message) = self.memory_context_message().await {
            messages.push(memory_message);
        }
        if let Some(conversation_message) = self.conversation_context_message().await {
            messages.push(conversation_message);
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: text.to_string(),
        });

        let request = self.get_chat_request(messages);

        let response = self.llm_gateway.chat(request).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        logger::debug(&format!("Agent Response: {}", content));

        let json_str = Self::extract_json(&content)?;
        let route_result: PlannerResult =
            serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;
        Ok(route_result)
    }

    /// 第二阶段：调用专家 Agent
    async fn call_worker(&self, role: AgentRole, text: &str) -> Result<PlannerResult, AIError> {
        self.emit_thinking(
            role.as_label(),
            "working",
            Some("专家开始处理当前请求".to_string()),
        )
        .await;

        let tools = self
            .skill_registry
            .for_role_and_platform(&role, &self.platform);
        let system_prompt = match role {
            AgentRole::TaskAgent => {
                PromptBuilder::build_task_agent_prompt(&self.preferred_lang, &tools)
            }
            AgentRole::HabitAgent => {
                PromptBuilder::build_habit_agent_prompt(&self.preferred_lang, &tools)
            }
            _ => "你是 Deadliner AI 助手。".to_string(),
        };

        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
        }];
        if let Some(memory_message) = self.memory_context_message().await {
            messages.push(memory_message);
        }
        if let Some(conversation_message) = self.conversation_context_message().await {
            messages.push(conversation_message);
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: text.to_string(),
        });

        let request = self.get_chat_request(messages);

        let response = self.llm_gateway.chat(request).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        logger::debug(&format!("Agent Response: {}", content));

        let worker_result: PlannerResult = match Self::extract_json(&content) {
            Ok(json_str) => serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?,
            Err(_) if role == AgentRole::ChatAgent => Self::fallback_chat_result(text, &content),
            Err(err) => return Err(err),
        };
        let mut normalized_result: MixedResult = worker_result.into();
        self.normalize_and_validate_tool_calls_guarded(&role, text, &mut normalized_result)?;
        Self::enforce_specialist_tool_only_contract(&role, &normalized_result)?;
        Self::normalize_query_result(&role, text, &mut normalized_result);
        if let Some(tool_calls) = &mut normalized_result.tool_calls {
            for tool_call in tool_calls.iter_mut() {
                tool_call.agent = Some(role.clone());
            }
        }
        Ok(PlannerResult {
            primary_intent: normalized_result.primary_intent,
            routed_agents: normalized_result.routed_agents,
            tasks: normalized_result.tasks,
            habits: normalized_result.habits,
            chat_response: normalized_result.chat_response,
            new_memories: normalized_result.new_memories,
            session_summary: normalized_result.session_summary,
            user_profile: normalized_result.user_profile,
            requested_capabilities: normalized_result.requested_capabilities,
            actions: normalized_result.actions,
            tool_calls: normalized_result.tool_calls,
        })
    }

    fn get_chat_request(&self, messages: Vec<ChatMessage>) -> ChatRequest {
        return ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: self.llm_gateway.get_temperature(),
        };
    }
}

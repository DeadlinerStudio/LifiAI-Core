impl Orchestrator {
    const TOOL_CONFIDENCE_THRESHOLD: usize = 3;

    fn tool_guard_fallback_message(role: &AgentRole, original_text: &str) -> String {
        let scope = match role {
            AgentRole::TaskAgent => "任务",
            AgentRole::HabitAgent => "习惯",
            _ => "当前请求",
        };
        format!(
            "我暂时不执行本地{}工具，以免产生误操作。请你确认一下需求后我再继续：{}",
            scope,
            original_text.trim()
        )
    }

    fn downgrade_invalid_tooling(
        role: &AgentRole,
        original_text: &str,
        result: &mut MixedResult,
        err: &AIError,
    ) {
        logger::debug(&format!("Tool guard downgrade triggered: {}", err));
        result.tool_calls = None;
        result.requested_capabilities = None;
        result.actions = None;
        if result
            .chat_response
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            result.chat_response = Some(Self::tool_guard_fallback_message(role, original_text));
        }
        result.primary_intent = Some("Chat".to_string());
    }

    fn normalize_and_validate_tool_calls_guarded(
        &self,
        role: &AgentRole,
        original_text: &str,
        result: &mut MixedResult,
    ) -> Result<(), AIError> {
        let normalized = self
            .normalize_requested_capabilities_to_tool_calls(role, result)
            .and_then(|_| self.normalize_actions_to_tool_calls(role, result))
            .and_then(|_| {
                if let Some(tool_calls) = &result.tool_calls {
                    self.validate_tool_calls(role, tool_calls)?;
                }
                Ok(())
            });

        match normalized {
            Ok(()) => {
                self.apply_tool_confidence_gate(role, original_text, result);
                Ok(())
            }
            Err(err @ AIError::InvalidToolCall(_)) | Err(err @ AIError::ConfigError(_)) => {
                Self::downgrade_invalid_tooling(role, original_text, result, &err);
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    fn apply_tool_confidence_gate(
        &self,
        role: &AgentRole,
        original_text: &str,
        result: &mut MixedResult,
    ) {
        let Some(tool_calls) = result.tool_calls.as_mut() else {
            return;
        };

        let is_query_request = Self::is_query_request(role, original_text);
        for tool_call in tool_calls.iter_mut() {
            let tool = match self
                .skill_registry
                .find_for_role_and_platform(role, &self.platform, &tool_call.tool)
            {
                Some(tool) => tool,
                None => continue,
            };

            let intent_match =
                Self::is_intent_match_for_role(role, result.primary_intent.as_deref(), original_text);
            let args_complete =
                Self::validate_args_against_schema(&tool_call.args, &tool.parameters, &tool.name)
                    .is_ok();
            let time_parse_success = Self::time_args_parse_success(&tool_call.args);
            let score = usize::from(intent_match)
                + usize::from(args_complete)
                + usize::from(time_parse_success)
                + usize::from(is_query_request);

            if score < Self::TOOL_CONFIDENCE_THRESHOLD {
                tool_call.execution_mode = Some(CapabilityExecutionMode::AskUser);
                let gate_reason = format!(
                    "[confidence_gate score={}/4 intent_match={} args_complete={} time_parse={} query={}]",
                    score, intent_match, args_complete, time_parse_success, is_query_request
                );
                tool_call.reason = Some(match &tool_call.reason {
                    Some(existing) if !existing.trim().is_empty() => {
                        format!("{} {}", existing, gate_reason)
                    }
                    _ => gate_reason,
                });
            }
        }
    }

    fn is_intent_match_for_role(
        role: &AgentRole,
        primary_intent: Option<&str>,
        original_text: &str,
    ) -> bool {
        if Self::is_query_request(role, original_text) {
            return true;
        }

        let intent = primary_intent.unwrap_or("").to_lowercase();
        match role {
            AgentRole::TaskAgent => {
                intent.contains("task") || intent.contains("任务") || intent.contains("extract")
            }
            AgentRole::HabitAgent => intent.contains("habit") || intent.contains("习惯"),
            AgentRole::ChatAgent => intent.contains("chat"),
            AgentRole::Supervisor => true,
        }
    }

    fn time_args_parse_success(args: &Value) -> bool {
        let mut found_time_like_value = false;

        fn walk(value: &Value, key_hint: Option<&str>, found_time_like_value: &mut bool) -> bool {
            match value {
                Value::Object(map) => map.iter().all(|(k, v)| walk(v, Some(k), found_time_like_value)),
                Value::Array(items) => items.iter().all(|item| walk(item, key_hint, found_time_like_value)),
                Value::String(raw) => {
                    let is_time_field = key_hint
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains("time")
                        || key_hint
                            .unwrap_or_default()
                            .to_ascii_lowercase()
                            .contains("date")
                        || key_hint.unwrap_or_default().to_ascii_lowercase().contains("due");

                    if !is_time_field {
                        return true;
                    }
                    *found_time_like_value = true;
                    chrono::NaiveDateTime::parse_from_str(raw, crate::domain::models::DEFAULT_TIME_FORMAT)
                        .is_ok()
                        || chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S").is_ok()
                        || chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").is_ok()
                        || chrono::DateTime::parse_from_rfc3339(raw).is_ok()
                }
                Value::Number(_) => true,
                Value::Bool(_) | Value::Null => true,
            }
        }

        let ok = walk(args, None, &mut found_time_like_value);
        if !found_time_like_value {
            true
        } else {
            ok
        }
    }

    fn read_context_capability(role: &AgentRole) -> Result<ToolCapability, AIError> {
        match role {
            AgentRole::TaskAgent => Ok(ToolCapability::ReadTaskContext),
            AgentRole::HabitAgent => Ok(ToolCapability::ReadHabitContext),
            _ => Err(AIError::ConfigError(format!(
                "{} 当前不支持 READ_CONTEXT capability 解析",
                role.as_label()
            ))),
        }
    }

    fn normalize_requested_capabilities_to_tool_calls(
        &self,
        role: &AgentRole,
        result: &mut MixedResult,
    ) -> Result<(), AIError> {
        let requests = result.requested_capabilities.take().unwrap_or_default();
        if requests.is_empty() {
            return Ok(());
        }

        // 新协议优先：当 requestedCapabilities 存在时，忽略同轮 model 直接给出的 toolCalls/actions
        let mut normalized_tool_calls = Vec::new();
        result.actions = None;

        for request in requests {
            let CapabilityRequest {
                capability,
                args,
                reason,
                execution_mode,
            } = request;
            let plan = self.skill_registry.resolve_execution_plan(
                role,
                &self.platform,
                &capability,
                args,
                execution_mode,
            )?;
            if matches!(plan.execution_mode, CapabilityExecutionMode::Never) {
                continue;
            }
            let tool = self
                .skill_registry
                .find_for_role_and_platform(role, &self.platform, &plan.tool_name)
                .ok_or_else(|| {
                    AIError::ConfigError(format!(
                        "无法根据 execution plan 回查工具 {}",
                        plan.tool_name
                    ))
                })?;
            Self::validate_args_against_schema(&plan.args, &tool.parameters, &tool.name)?;
            normalized_tool_calls.push(AIToolCall {
                agent: Some(role.clone()),
                tool: plan.tool_name,
                args: plan.args,
                reason,
                execution_mode: Some(plan.execution_mode),
            });
        }

        result.tool_calls = if normalized_tool_calls.is_empty() {
            None
        } else {
            Some(normalized_tool_calls)
        };

        Ok(())
    }

    fn normalize_actions_to_tool_calls(
        &self,
        role: &AgentRole,
        result: &mut MixedResult,
    ) -> Result<(), AIError> {
        if result
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        {
            return Ok(());
        }

        let actions = result.actions.take().unwrap_or_default();
        if actions.is_empty() {
            return Ok(());
        }

        let mut normalized_tool_calls = result.tool_calls.take().unwrap_or_default();

        for action in actions {
            let AIActionCall {
                action,
                args,
                reason,
            } = action;
            match action.as_str() {
                "READ_CONTEXT" => {
                    let capability = Self::read_context_capability(role)?;
                    let plan = self.skill_registry.resolve_execution_plan(
                        role,
                        &self.platform,
                        &capability,
                        args,
                        None,
                    )?;
                    let tool = self
                        .skill_registry
                        .find_for_role_and_platform(role, &self.platform, &plan.tool_name)
                        .ok_or_else(|| {
                            AIError::ConfigError(format!(
                                "无法根据 execution plan 回查工具 {}",
                                plan.tool_name
                            ))
                        })?;
                    Self::validate_args_against_schema(&plan.args, &tool.parameters, &tool.name)?;
                    normalized_tool_calls.push(AIToolCall {
                        agent: Some(role.clone()),
                        tool: plan.tool_name,
                        args: plan.args,
                        reason,
                        execution_mode: None,
                    });
                }
                other => {
                    return Err(AIError::InvalidToolCall(format!(
                        "未知 action {}，当前仅支持 READ_CONTEXT",
                        other
                    )));
                }
            }
        }

        result.tool_calls = if normalized_tool_calls.is_empty() {
            None
        } else {
            Some(normalized_tool_calls)
        };

        Ok(())
    }

    fn validate_tool_calls(
        &self,
        role: &AgentRole,
        tool_calls: &[AIToolCall],
    ) -> Result<(), AIError> {
        for tool_call in tool_calls {
            let tool = self
                .skill_registry
                .find_for_role_and_platform(role, &self.platform, &tool_call.tool)
                .ok_or_else(|| {
                    AIError::InvalidToolCall(format!(
                        "工具 {} 不属于 {} 或当前平台 {} 不支持",
                        tool_call.tool,
                        role.as_label(),
                        self.platform
                    ))
                })?;

            Self::validate_args_against_schema(&tool_call.args, &tool.parameters, &tool.name)?;
        }

        Ok(())
    }

    fn validate_args_against_schema(
        args: &Value,
        schema: &Value,
        tool_name: &str,
    ) -> Result<(), AIError> {
        let expected_type = schema
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("object");

        if expected_type == "object" {
            let args_object = args.as_object().ok_or_else(|| {
                AIError::InvalidToolCall(format!("工具 {} 的 args 必须是 object", tool_name))
            })?;

            let required_fields = schema
                .get("required")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            for field in required_fields {
                if let Some(field_name) = field.as_str() {
                    if !args_object.contains_key(field_name) {
                        return Err(AIError::InvalidToolCall(format!(
                            "工具 {} 缺少必填字段 {}",
                            tool_name, field_name
                        )));
                    }
                }
            }

            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();

            for (key, value) in args_object {
                if let Some(property_schema) = properties.get(key) {
                    Self::validate_value_type(value, property_schema, tool_name, key)?;
                }
            }
            return Ok(());
        }

        Self::validate_value_type(args, schema, tool_name, "args")
    }

    fn validate_value_type(
        value: &Value,
        schema: &Value,
        tool_name: &str,
        field_name: &str,
    ) -> Result<(), AIError> {
        let expected_type = schema.get("type").and_then(Value::as_str);
        match expected_type {
            Some("string") if !value.is_string() => Err(AIError::InvalidToolCall(format!(
                "工具 {} 的字段 {} 必须是 string",
                tool_name, field_name
            ))),
            Some("integer") if value.as_i64().is_none() && value.as_u64().is_none() => {
                Err(AIError::InvalidToolCall(format!(
                    "工具 {} 的字段 {} 必须是 integer",
                    tool_name, field_name
                )))
            }
            Some("number") if !value.is_number() => Err(AIError::InvalidToolCall(format!(
                "工具 {} 的字段 {} 必须是 number",
                tool_name, field_name
            ))),
            Some("boolean") if !value.is_boolean() => Err(AIError::InvalidToolCall(format!(
                "工具 {} 的字段 {} 必须是 boolean",
                tool_name, field_name
            ))),
            Some("array") => {
                let items = value.as_array().ok_or_else(|| {
                    AIError::InvalidToolCall(format!(
                        "工具 {} 的字段 {} 必须是 array",
                        tool_name, field_name
                    ))
                })?;
                if let Some(item_schema) = schema.get("items") {
                    for item in items {
                        Self::validate_value_type(item, item_schema, tool_name, field_name)?;
                    }
                }
                Ok(())
            }
            Some("object") if !value.is_object() => Err(AIError::InvalidToolCall(format!(
                "工具 {} 的字段 {} 必须是 object",
                tool_name, field_name
            ))),
            _ => Ok(()),
        }
    }

    fn observation_task_fallback(result_json: &str) -> Option<Vec<crate::domain::models::AITask>> {
        let value: Value = serde_json::from_str(result_json).ok()?;
        let tasks = value.get("tasks")?.as_array()?;
        if tasks.is_empty() {
            return Some(Vec::new());
        }

        let mapped: Vec<crate::domain::models::AITask> = tasks
            .iter()
            .filter_map(|task| {
                let name = task.get("name")?.as_str()?.trim().to_string();
                if name.is_empty() {
                    return None;
                }
                let due_time = task
                    .get("due")
                    .and_then(Value::as_str)
                    .map(|raw| raw.replace('T', " "));
                let note = task
                    .get("notePreview")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);

                Some(crate::domain::models::AITask {
                    name,
                    due_time,
                    note,
                })
            })
            .collect();

        Some(mapped)
    }

    fn apply_observation_fallback(
        role: &AgentRole,
        result_json: &str,
        worker_result: &mut MixedResult,
    ) {
        if matches!(role, AgentRole::TaskAgent) {
            let needs_fallback = worker_result
                .tasks
                .as_ref()
                .map(|tasks| tasks.is_empty())
                .unwrap_or(true);
            if needs_fallback {
                if let Some(tasks) = Self::observation_task_fallback(result_json) {
                    if !tasks.is_empty() {
                        worker_result.tasks = Some(tasks);
                    }
                }
            }
        }
    }

}

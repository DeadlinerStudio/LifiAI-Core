use async_trait::async_trait;
use futures::future::join_all;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::domain::models::{MixedResult, AIError, ChatRequest, ChatMessage, AgentRole, AIToolCall};
use crate::domain::traits::{LlmGateway, AgentOrchestrator, ToolExecutor, MemoryStore};
use crate::application::prompt_builder::PromptBuilder;
use crate::infrastructure::skill_registry::SkillRegistry;

#[derive(Debug, Clone)]
struct PendingToolContext {
    role: AgentRole,
    original_text: String,
}

pub struct Orchestrator {
    llm_gateway: Arc<dyn LlmGateway>,
    memory_store: Arc<dyn MemoryStore>,
    skill_registry: Arc<SkillRegistry>,
    platform: String,
    pending_tool_calls: Arc<Mutex<HashMap<String, PendingToolContext>>>,
    pub preferred_lang: String,
}

impl Orchestrator {
    pub fn new(
        llm_gateway: Arc<dyn LlmGateway>,
        memory_store: Arc<dyn MemoryStore>,
        skill_registry: Arc<SkillRegistry>,
        platform: String,
        preferred_lang: String,
    ) -> Self {
        Self {
            llm_gateway,
            memory_store,
            skill_registry,
            platform,
            pending_tool_calls: Arc::new(Mutex::new(HashMap::new())),
            preferred_lang,
        }
    }

    /// 清理并提取 JSON 字符串
    fn extract_json(raw: &str) -> Result<String, AIError> {
        let mut cleaned = raw.trim();
        if cleaned.starts_with("```json") {
            cleaned = cleaned.strip_prefix("```json").unwrap_or(cleaned);
        } else if cleaned.starts_with("```") {
            cleaned = cleaned.strip_prefix("```").unwrap_or(cleaned);
        }
        if cleaned.ends_with("```") {
            cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned);
        }
        let cleaned = cleaned.trim();

        if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
            Ok(cleaned[start..=end].to_string())
        } else {
            Err(AIError::ParsingFailed)
        }
    }

    fn validate_tool_calls(&self, role: &AgentRole, tool_calls: &[AIToolCall]) -> Result<(), AIError> {
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

    async fn memory_context_message(&self) -> Option<ChatMessage> {
        match self.memory_store.get_long_term_context().await {
            Ok(context) if !context.trim().is_empty() => Some(ChatMessage {
                role: "system".to_string(),
                content: format!("以下是用户的长期记忆上下文，请在推理时参考：\n{}", context),
            }),
            _ => None,
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

    pub async fn register_pending_tool_call(
        &self,
        tool_call_id: String,
        role: AgentRole,
        original_text: String,
    ) {
        let mut pending = self.pending_tool_calls.lock().await;
        pending.insert(
            tool_call_id,
            PendingToolContext {
                role,
                original_text,
            },
        );
    }

    async fn continue_with_tool_observation(
        &self,
        role: AgentRole,
        original_text: String,
        result_json: String,
    ) -> Result<MixedResult, AIError> {
        let tools = self.skill_registry.for_role_and_platform(&role, &self.platform);
        let system_prompt = match role {
            AgentRole::TaskAgent => PromptBuilder::build_task_agent_prompt(&self.preferred_lang, &tools),
            AgentRole::HabitAgent => PromptBuilder::build_habit_agent_prompt(&self.preferred_lang, &tools),
            _ => "你是 Deadliner AI 助手。".to_string(),
        };

        let observation_prompt = format!(
            "原始用户输入：{}\n\n工具执行结果 JSON：{}\n\n请结合上述 observation 输出最终 JSON。若信息已经足够，不要再次输出 toolCalls。",
            original_text, result_json
        );

        let mut messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt },
        ];
        if let Some(memory_message) = self.memory_context_message().await {
            messages.push(memory_message);
        }
        messages.push(ChatMessage { role: "user".to_string(), content: observation_prompt });

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        let response = self.llm_gateway.chat(request).await?;
        let content = response.choices.first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        println!("DEBUG: Observation Response: {}", content);

        let json_str = Self::extract_json(&content)?;
        let worker_result: MixedResult = serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;
        if let Some(tool_calls) = &worker_result.tool_calls {
            self.validate_tool_calls(&role, tool_calls)?;
        }
        self.persist_memory_updates(&worker_result).await?;
        Ok(worker_result)
    }

    /// 第一阶段：Supervisor 路由
    async fn route(&self, text: &str) -> Result<MixedResult, AIError> {
        let system_prompt = PromptBuilder::build_supervisor_prompt(&self.preferred_lang);
        let mut messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt },
        ];
        if let Some(memory_message) = self.memory_context_message().await {
            messages.push(memory_message);
        }
        messages.push(ChatMessage { role: "user".to_string(), content: text.to_string() });

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        let response = self.llm_gateway.chat(request).await?;
        let content = response.choices.first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        println!("DEBUG: Agent Response: {}", content);

        let json_str = Self::extract_json(&content)?;
        let route_result: MixedResult = serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;
        Ok(route_result)
    }

    /// 第二阶段：调用专家 Agent
    async fn call_worker(&self, role: AgentRole, text: &str) -> Result<MixedResult, AIError> {
        let tools = self.skill_registry.for_role_and_platform(&role, &self.platform);
        let system_prompt = match role {
            AgentRole::TaskAgent => PromptBuilder::build_task_agent_prompt(&self.preferred_lang, &tools),
            AgentRole::HabitAgent => PromptBuilder::build_habit_agent_prompt(&self.preferred_lang, &tools),
            _ => "你是 Deadliner AI 助手。".to_string(),
        };

        let mut messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt },
        ];
        if let Some(memory_message) = self.memory_context_message().await {
            messages.push(memory_message);
        }
        messages.push(ChatMessage { role: "user".to_string(), content: text.to_string() });

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        let response = self.llm_gateway.chat(request).await?;
        let content = response.choices.first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        println!("DEBUG: Agent Response: {}", content);

        let json_str = Self::extract_json(&content)?;
        let mut worker_result: MixedResult = serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;
        if let Some(tool_calls) = &mut worker_result.tool_calls {
            self.validate_tool_calls(&role, tool_calls)?;
            for tool_call in tool_calls.iter_mut() {
                tool_call.agent = Some(role.clone());
            }
        }
        Ok(worker_result)
    }
}

#[async_trait]
impl AgentOrchestrator for Orchestrator {
    async fn process_message(&self, text: String) -> Result<MixedResult, AIError> {
        // 1. 第一阶段：路由
        let route_info = self.route(&text).await?;
        let routed_agents = route_info.routed_agents.clone().unwrap_or_default();

        if routed_agents.is_empty() {
             return Ok(MixedResult {
                 chat_response: Some("抱歉，我没太明白您的意思。".to_string()),
                 ..Default::default()
             });
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
            if let Some(tool_calls) = worker_res.tool_calls {
                let mut current_tool_calls = final_result.tool_calls.unwrap_or_default();
                current_tool_calls.extend(tool_calls);
                final_result.tool_calls = Some(current_tool_calls);
            }
        }

        self.persist_memory_updates(&final_result).await?;
        Ok(final_result)
    }
}

#[async_trait]
impl ToolExecutor for Orchestrator {
    async fn submit_tool_result(&self, tool_call_id: String, result_json: String) -> Result<MixedResult, AIError> {
        let context = {
            let mut pending = self.pending_tool_calls.lock().await;
            pending.remove(&tool_call_id)
        }
        .ok_or_else(|| AIError::InvalidToolCall(format!("未找到待回灌的工具调用 {}", tool_call_id)))?;

        self.continue_with_tool_observation(context.role, context.original_text, result_json).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use serde_json::json;

    use crate::domain::models::{
        ChatChoice, ChatResponse, MemoryFragment, MemorySnapshot, MemorySyncPayload, ToolMetadata,
    };

    struct MockLlmGateway {
        model: String,
        responses: Mutex<VecDeque<String>>,
    }

    impl MockLlmGateway {
        fn new(responses: Vec<String>) -> Self {
            Self {
                model: "mock-model".to_string(),
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    #[async_trait]
    impl LlmGateway for MockLlmGateway {
        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, AIError> {
            let mut responses = self.responses.lock().await;
            let content = responses
                .pop_front()
                .ok_or_else(|| AIError::NetworkError("mock responses exhausted".to_string()))?;

            Ok(ChatResponse {
                choices: vec![ChatChoice {
                    message: ChatMessage {
                        role: "assistant".to_string(),
                        content,
                    },
                }],
            })
        }

        fn get_model_name(&self) -> String {
            self.model.clone()
        }
    }

    struct MockMemoryStore;

    #[async_trait]
    impl MemoryStore for MockMemoryStore {
        async fn save_memory(&self, _content: String, _category: String) -> Result<(), AIError> {
            Ok(())
        }

        async fn get_all_fragments(&self) -> Result<Vec<MemoryFragment>, AIError> {
            Ok(vec![])
        }

        async fn save_user_profile(&self, _profile: String) -> Result<(), AIError> {
            Ok(())
        }

        async fn get_user_profile(&self) -> Result<String, AIError> {
            Ok(String::new())
        }

        async fn get_long_term_context(&self) -> Result<String, AIError> {
            Ok(String::new())
        }

        async fn replace_snapshot(&self, _snapshot: MemorySnapshot) -> Result<(), AIError> {
            Ok(())
        }

        async fn current_snapshot(&self) -> Result<MemorySnapshot, AIError> {
            Ok(MemorySnapshot::default())
        }

        async fn drain_pending_sync(&self) -> Result<MemorySyncPayload, AIError> {
            Ok(MemorySyncPayload::default())
        }
    }

    fn build_test_registry() -> Arc<SkillRegistry> {
        Arc::new(SkillRegistry::from_skills(vec![ToolMetadata {
            name: "read_tasks".to_string(),
            description: "读取任务".to_string(),
            domain: AgentRole::TaskAgent,
            supported_platforms: vec!["ios".to_string(), "android".to_string()],
            requires_ui_confirmation: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "timeRangeDays": { "type": "integer" },
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                }
            }),
        }]))
    }

    #[tokio::test]
    async fn process_message_and_submit_tool_result_form_a_local_mock_loop() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "toolCalls": [
                    {
                        "tool": "read_tasks",
                        "args": {
                            "timeRangeDays": 7,
                            "keywords": ["Rust", "冒烟测试"]
                        },
                        "reason": "需要检查是否已有重复任务"
                    }
                ]
            })
            .to_string(),
            json!({
                "tasks": [
                    {
                        "name": "写 Rust 冒烟测试",
                        "dueTime": "2026-03-19 10:00",
                        "note": "避免重复后确认创建"
                    }
                ]
            })
            .to_string(),
        ]));

        let memory_store: Arc<dyn MemoryStore> = Arc::new(MockMemoryStore);
        let orchestrator = Orchestrator::new(
            llm_gateway,
            memory_store,
            build_test_registry(),
            "ios".to_string(),
            "zh-CN".to_string(),
        );

        let user_input = "帮我记一下明天上午十点写 Rust 冒烟测试".to_string();
        let initial_result = orchestrator.process_message(user_input.clone()).await.unwrap();

        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected one tool call");
        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.agent, Some(AgentRole::TaskAgent));

        let tool_call_id = "tool-call-1".to_string();
        orchestrator
            .register_pending_tool_call(tool_call_id.clone(), AgentRole::TaskAgent, user_input)
            .await;

        let final_result = orchestrator
            .submit_tool_result(
                tool_call_id,
                json!({
                    "tasks": []
                })
                .to_string(),
            )
            .await
            .unwrap();

        let tasks = final_result.tasks.expect("expected extracted tasks");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "写 Rust 冒烟测试");
        assert_eq!(tasks[0].due_time.as_deref(), Some("2026-03-19 10:00"));
    }

    #[tokio::test]
    async fn rejects_invalid_tool_call_args_from_mock_response() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "toolCalls": [
                    {
                        "tool": "read_tasks",
                        "args": {
                            "timeRangeDays": "seven"
                        },
                        "reason": "参数故意写错"
                    }
                ]
            })
            .to_string(),
        ]));

        let memory_store: Arc<dyn MemoryStore> = Arc::new(MockMemoryStore);
        let orchestrator = Orchestrator::new(
            llm_gateway,
            memory_store,
            build_test_registry(),
            "ios".to_string(),
            "zh-CN".to_string(),
        );

        let err = orchestrator
            .process_message("帮我看看下周任务".to_string())
            .await
            .expect_err("expected invalid tool call to fail");

        match err {
            AIError::InvalidToolCall(message) => {
                assert!(message.contains("timeRangeDays"));
            }
            other => panic!("unexpected error: {}", other),
        }
    }
}

use crate::application::prompt_builder::PromptBuilder;
use crate::domain::models::{
    AIActionCall, AIError, AIToolCall, AgentRole, CapabilityExecutionMode, CapabilityRequest,
    ChatMessage, ChatRequest, MixedResult, PlannerResult, ToolCapability,
};
use crate::domain::traits::{AgentOrchestrator, LlmGateway, MemoryStore, ToolExecutor};
use crate::infrastructure::skill_registry::SkillRegistry;
use async_trait::async_trait;
use futures::future::join_all;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait OrchestratorProgressSink: Send + Sync {
    async fn on_thinking(&self, agent_name: String, phase: String, message: Option<String>);
}

#[derive(Debug, Clone, Default)]
pub struct MemoryCommitResult {
    pub added_memories: Vec<String>,
    pub profile_updated: bool,
    pub new_revision: u64,
    pub memory_sync_json: Option<String>,
}

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
    progress_sink: Arc<Mutex<Option<Arc<dyn OrchestratorProgressSink>>>>,
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
            progress_sink: Arc::new(Mutex::new(None)),
            preferred_lang,
        }
    }

    pub async fn set_progress_sink(&self, sink: Arc<dyn OrchestratorProgressSink>) {
        let mut current = self.progress_sink.lock().await;
        *current = Some(sink);
    }

    async fn emit_thinking(&self, agent_name: &str, phase: &str, message: Option<String>) {
        let sink = {
            let current = self.progress_sink.lock().await;
            current.clone()
        };

        if let Some(sink) = sink {
            sink.on_thinking(agent_name.to_string(), phase.to_string(), message)
                .await;
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

    fn fallback_chat_result(text: &str, content: &str) -> PlannerResult {
        let trimmed_text = text.trim();
        let trimmed_content = content.trim();

        let mut result = PlannerResult {
            primary_intent: Some("Chat".to_string()),
            chat_response: Some(trimmed_content.to_string()),
            ..Default::default()
        };

        if Self::looks_like_memory_statement(trimmed_text) {
            result.new_memories = Some(vec![trimmed_text.to_string()]);
        }

        if Self::looks_like_profile_statement(trimmed_text) {
            result.user_profile = Some(trimmed_text.to_string());
        }

        result
    }

    fn looks_like_memory_statement(text: &str) -> bool {
        let patterns = [
            "记住",
            "记一下",
            "以后",
            "我是",
            "我喜欢",
            "我习惯",
            "我通常",
            "请叫我",
        ];
        patterns.iter().any(|pattern| text.contains(pattern))
    }

    fn looks_like_profile_statement(text: &str) -> bool {
        let patterns = [
            "我是",
            "我喜欢",
            "我习惯",
            "我通常",
            "偏好",
            "请叫我",
            "默认",
        ];
        patterns.iter().any(|pattern| text.contains(pattern))
    }

    fn looks_like_task_query(text: &str) -> bool {
        let normalized = text.to_lowercase();
        let exclusion_patterns = ["重复", "冲突", "是否已有", "有没有重复", "避免重复", "去重"];
        let direct_patterns = [
            "有哪些任务",
            "有什么任务",
            "任务列表",
            "列出任务",
            "看看任务",
            "查看任务",
            "总结任务",
            "哪些待办",
            "有什么待办",
            "最近有什么任务",
            "这周有什么任务",
            "本周有什么任务",
            "read my tasks",
            "show my tasks",
            "list my tasks",
            "view my tasks",
            "what are my tasks",
            "what's on my task list",
            "task list",
            "my tasks",
        ];
        let query_verbs = [
            "列出",
            "查看",
            "看看",
            "总结",
            "查询",
            "盘点",
            "read",
            "show",
            "list",
            "view",
            "summarize",
        ];
        let task_nouns = ["任务", "待办", "deadline", "ddl", "todo", "task", "tasks"];

        if exclusion_patterns
            .iter()
            .any(|pattern| normalized.contains(pattern))
        {
            return false;
        }

        direct_patterns
            .iter()
            .any(|pattern| normalized.contains(pattern))
            || (query_verbs.iter().any(|verb| normalized.contains(verb))
                && task_nouns.iter().any(|noun| normalized.contains(noun)))
    }

    fn looks_like_habit_query(text: &str) -> bool {
        let normalized = text.to_lowercase();
        let patterns = [
            "有哪些习惯",
            "我的习惯",
            "习惯列表",
            "查看习惯",
            "看看习惯",
            "总结习惯",
        ];
        patterns.iter().any(|pattern| normalized.contains(pattern))
    }

    fn is_query_request(role: &AgentRole, text: &str) -> bool {
        match role {
            AgentRole::TaskAgent => Self::looks_like_task_query(text),
            AgentRole::HabitAgent => Self::looks_like_habit_query(text),
            _ => false,
        }
    }

    fn summarize_tasks_for_query(tasks: &[crate::domain::models::AITask]) -> String {
        if tasks.is_empty() {
            return "当前没有查到相关任务。".to_string();
        }

        let summary = tasks
            .iter()
            .take(5)
            .map(|task| match task.due_time.as_deref() {
                Some(due) if !due.is_empty() => format!("{}（{}）", task.name, due),
                _ => task.name.clone(),
            })
            .collect::<Vec<_>>()
            .join("；");

        let suffix = if tasks.len() > 5 {
            "；其余任务已省略。"
        } else {
            "。"
        };
        format!(
            "我查到你当前有 {} 个相关任务：{}{}",
            tasks.len(),
            summary,
            suffix
        )
    }

    fn summarize_habits_for_query(habits: &[crate::domain::models::AIHabit]) -> String {
        if habits.is_empty() {
            return "当前没有查到相关习惯。".to_string();
        }

        let summary = habits
            .iter()
            .take(5)
            .map(|habit| format!("{}（{}）", habit.name, habit.period))
            .collect::<Vec<_>>()
            .join("；");

        let suffix = if habits.len() > 5 {
            "；其余习惯已省略。"
        } else {
            "。"
        };
        format!(
            "我查到你当前有 {} 个相关习惯：{}{}",
            habits.len(),
            summary,
            suffix
        )
    }

    fn normalize_query_result(role: &AgentRole, original_text: &str, result: &mut MixedResult) {
        if !Self::is_query_request(role, original_text) {
            return;
        }

        match role {
            AgentRole::TaskAgent => {
                let tasks = result.tasks.take().unwrap_or_default();
                if result
                    .chat_response
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    result.chat_response = Some(Self::summarize_tasks_for_query(&tasks));
                }
                result.retrieved_tasks = Some(tasks);
                result.tasks = Some(vec![]);
            }
            AgentRole::HabitAgent => {
                let habits = result.habits.take().unwrap_or_default();
                if result
                    .chat_response
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    result.chat_response = Some(Self::summarize_habits_for_query(&habits));
                }
                result.retrieved_habits = Some(habits);
                result.habits = Some(vec![]);
            }
            _ => {}
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

        let mut normalized_tool_calls = result.tool_calls.take().unwrap_or_default();

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
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: synthesis_prompt,
        });

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        let response = self.llm_gateway.chat(request).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        println!("DEBUG: Memory Synthesis Response: {}", content);

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
6. 只输出纯 JSON。"#,
            original_text, result_json
        );

        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
        }];
        if let Some(memory_message) = self.memory_context_message().await {
            messages.push(memory_message);
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: observation_prompt,
        });

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        let response = self.llm_gateway.chat(request).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        println!("DEBUG: Observation Response: {}", content);

        let json_str = Self::extract_json(&content)?;
        let mut worker_result: MixedResult =
            serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;
        self.normalize_requested_capabilities_to_tool_calls(&role, &mut worker_result)?;
        self.normalize_actions_to_tool_calls(&role, &mut worker_result)?;
        Self::apply_observation_fallback(&role, &result_json, &mut worker_result);
        Self::normalize_query_result(&role, &original_text, &mut worker_result);
        if let Some(tool_calls) = &worker_result.tool_calls {
            self.validate_tool_calls(&role, tool_calls)?;
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
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: text.to_string(),
        });

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        let response = self.llm_gateway.chat(request).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        println!("DEBUG: Agent Response: {}", content);

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
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: text.to_string(),
        });

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        let response = self.llm_gateway.chat(request).await?;
        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        println!("DEBUG: Agent Response: {}", content);

        let worker_result: PlannerResult = match Self::extract_json(&content) {
            Ok(json_str) => serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?,
            Err(_) if role == AgentRole::ChatAgent => Self::fallback_chat_result(text, &content),
            Err(err) => return Err(err),
        };
        let mut normalized_result: MixedResult = worker_result.into();
        self.normalize_requested_capabilities_to_tool_calls(&role, &mut normalized_result)?;
        self.normalize_actions_to_tool_calls(&role, &mut normalized_result)?;
        Self::normalize_query_result(&role, text, &mut normalized_result);
        if let Some(tool_calls) = &mut normalized_result.tool_calls {
            self.validate_tool_calls(&role, tool_calls)?;
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
}

impl Orchestrator {
    pub async fn process_message_core(&self, text: String) -> Result<MixedResult, AIError> {
        // 1. 第一阶段：路由
        let route_info = self.route(&text).await?;
        let routed_agents = route_info.routed_agents.clone().unwrap_or_default();

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

        let original_text = context.original_text;
        let mut result = self
            .continue_with_tool_observation(context.role, original_text.clone(), result_json)
            .await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use serde_json::json;

    use crate::domain::models::{
        ChatChoice, ChatResponse, MemoryFragment, MemorySnapshot, MemorySyncPayload,
        ToolCapability, ToolMetadata,
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
            capabilities: vec![ToolCapability::ReadTaskContext],
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
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备执行任务去重工具",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "完成了一次任务去重检查",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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
        let initial_result = orchestrator
            .process_message(user_input.clone())
            .await
            .unwrap();

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
    async fn legacy_action_is_still_normalized_into_registered_read_tool() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "actions": [
                    {
                        "action": "READ_CONTEXT",
                        "args": {
                            "timeRangeDays": 7,
                            "keywords": ["本周"]
                        },
                        "reason": "回答前先读取本地任务上下文"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let result = orchestrator
            .process_message("那我这周有什么任务呀？".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected normalized tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(7));
        assert_eq!(tool_call.args["keywords"], json!(["本周"]));
        assert_eq!(tool_call.agent, Some(AgentRole::TaskAgent));
    }

    #[tokio::test]
    async fn requested_capability_is_normalized_into_registered_read_tool() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 7,
                            "keywords": ["本周"]
                        },
                        "reason": "回答前先请求任务上下文"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let result = orchestrator
            .process_message("那我这周有什么任务呀？".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected normalized tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(7));
        assert_eq!(tool_call.args["keywords"], json!(["本周"]));
        assert_eq!(tool_call.agent, Some(AgentRole::TaskAgent));
    }

    #[tokio::test]
    async fn requested_capability_defaults_to_auto_for_single_non_confirm_tool() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 5
                        },
                        "reason": "读取任务上下文"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let result = orchestrator
            .process_message("看看我的任务".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(5));
    }

    #[tokio::test]
    async fn requested_capability_with_never_does_not_emit_tool_call() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "chatResponse": "我暂时不读取本地任务，先根据已有信息回答。",
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "executionMode": "NEVER",
                        "args": {
                            "timeRangeDays": 7
                        },
                        "reason": "本轮禁止读取工具"
                    }
                ],
                "tasks": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "未读取任务上下文",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let result = orchestrator
            .process_message("看看我的任务".to_string())
            .await
            .unwrap();

        assert!(
            result
                .tool_calls
                .as_ref()
                .map(|calls| calls.is_empty())
                .unwrap_or(true)
        );
        assert_eq!(
            result.chat_response.as_deref(),
            Some("我暂时不读取本地任务，先根据已有信息回答。")
        );
    }

    #[tokio::test]
    async fn requested_capabilities_take_precedence_over_legacy_actions() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 14
                        },
                        "reason": "优先使用新协议"
                    }
                ],
                "actions": [
                    {
                        "action": "READ_CONTEXT",
                        "args": {
                            "timeRangeDays": 3
                        },
                        "reason": "旧协议不应覆盖新协议"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let result = orchestrator
            .process_message("看看我的任务".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected normalized tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(14));
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
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": null,
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

    #[tokio::test]
    async fn rejects_unknown_action_from_mock_response() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "actions": [
                    {
                        "action": "READ_TASK_LIST",
                        "args": {
                            "timeRangeDays": 7
                        }
                    }
                ]
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": null,
                "tasks": [],
                "habits": [],
                "toolCalls": []
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
            .process_message("帮我看看这周任务".to_string())
            .await
            .expect_err("expected invalid action to fail");

        match err {
            AIError::InvalidToolCall(message) => {
                assert!(message.contains("READ_TASK_LIST"));
            }
            other => panic!("unexpected error: {}", other),
        }
    }

    #[tokio::test]
    async fn observation_fallback_uses_tool_tasks_when_model_returns_empty() {
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
                            "timeRangeDays": 7
                        },
                        "reason": "检查已有任务"
                    }
                ]
            })
            .to_string(),
            json!({
                "tasks": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "读取了已有任务并整理结果",
                "tasks": [],
                "habits": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "本轮没有新增长期记忆",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let user_input = "看看有没有重复任务".to_string();
        let initial_result = orchestrator
            .process_message(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        orchestrator
            .register_pending_tool_call(
                "fallback-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result(
                "fallback-tool-call".to_string(),
                json!({
                    "summary": {
                        "count": 2,
                        "dueSoon24h": 0,
                        "overdue": 0
                    },
                    "tasks": [
                        {
                            "notePreview": "用户提及的会议",
                            "name": "开会",
                            "id": 2,
                            "due": "2026-03-20T15:00:00",
                            "status": "OPEN"
                        },
                        {
                            "notePreview": "",
                            "name": "11111",
                            "id": 1,
                            "due": "2026-03-28T00:22:00",
                            "status": "OPEN"
                        }
                    ]
                })
                .to_string(),
            )
            .await
            .unwrap();

        let tasks = final_result.tasks.expect("expected fallback tasks");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].name, "开会");
        assert_eq!(tasks[0].due_time.as_deref(), Some("2026-03-20 15:00:00"));
        assert_eq!(tasks[1].name, "11111");
    }

    #[tokio::test]
    async fn task_query_returns_chat_response_instead_of_task_proposals() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 7
                        },
                        "reason": "需要先请求近期任务上下文"
                    }
                ]
            })
            .to_string(),
            json!({
                "tasks": [
                    {
                        "name": "开会",
                        "dueTime": "2026-03-20 15:00",
                        "note": "用户提及的会议"
                    },
                    {
                        "name": "准备后天下午3点的开发会议",
                        "dueTime": "2026-03-21 15:00",
                        "note": "需要提前准备会议议程和材料"
                    }
                ],
                "requestedCapabilities": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "用户查看了任务列表",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let user_input = "那我这周有什么任务呀？".to_string();
        let initial_result = orchestrator
            .process_message_core(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        orchestrator
            .register_pending_tool_call(
                "query-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result_core(
                "query-tool-call".to_string(),
                json!({
                    "tasks": [
                        {
                            "notePreview": "用户提及的会议",
                            "name": "开会",
                            "id": 2,
                            "due": "2026-03-20T15:00:00",
                            "status": "OPEN"
                        },
                        {
                            "notePreview": "",
                            "name": "准备后天下午3点的开发会议",
                            "id": 1,
                            "due": "2026-03-21T15:00:00",
                            "status": "OPEN"
                        }
                    ],
                    "summary": {
                        "count": 2,
                        "dueSoon24h": 0,
                        "overdue": 0
                    }
                })
                .to_string(),
            )
            .await
            .unwrap()
            .1;

        assert_eq!(final_result.primary_intent.as_deref(), Some("Chat"));
        assert_eq!(
            final_result.tasks.as_ref().map(|tasks| tasks.len()),
            Some(0)
        );
        assert_eq!(
            final_result
                .retrieved_tasks
                .as_ref()
                .map(|tasks| tasks.len()),
            Some(2)
        );
        assert!(
            final_result
                .chat_response
                .as_deref()
                .unwrap_or("")
                .contains("2 个相关任务")
        );
    }

    #[tokio::test]
    async fn english_task_query_returns_chat_response_instead_of_task_proposals() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 7
                        },
                        "reason": "Need to request recent task context first."
                    }
                ]
            })
            .to_string(),
            json!({
                "tasks": [],
                "chatResponse": "I found 3 tasks for you.",
                "requestedCapabilities": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "User viewed the task list.",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let user_input = "Read my tasks".to_string();
        let initial_result = orchestrator
            .process_message_core(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        orchestrator
            .register_pending_tool_call(
                "english-query-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result_core(
                "english-query-tool-call".to_string(),
                json!({
                    "tasks": [
                        {
                            "notePreview": "用户提及的会议",
                            "name": "开会",
                            "id": 2,
                            "due": "2026-03-20T15:00:00",
                            "status": "OPEN"
                        },
                        {
                            "notePreview": "需要提前准备会议议程和材料",
                            "name": "准备后天下午3点的开发会议",
                            "id": 1,
                            "due": "2026-03-21T15:00:00",
                            "status": "OPEN"
                        }
                    ],
                    "summary": {
                        "count": 2,
                        "dueSoon24h": 0,
                        "overdue": 0
                    }
                })
                .to_string(),
            )
            .await
            .unwrap()
            .1;

        assert_eq!(final_result.primary_intent.as_deref(), Some("Chat"));
        assert_eq!(
            final_result.tasks.as_ref().map(|tasks| tasks.len()),
            Some(0)
        );
        assert_eq!(
            final_result
                .retrieved_tasks
                .as_ref()
                .map(|tasks| tasks.len()),
            Some(2)
        );
        assert!(
            final_result
                .chat_response
                .as_deref()
                .unwrap_or("")
                .contains("I found 3 tasks")
        );
    }

    #[tokio::test]
    async fn final_memory_synthesizer_adds_memories_to_merged_result() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "tasks": [
                    {
                        "name": "写 Rust 冒烟测试",
                        "dueTime": "2026-03-19 10:00",
                        "note": "明天上午十点"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": ["用户最近在推进 Rust 测试相关开发"],
                "userProfile": "用户当前在进行 Rust 工程开发",
                "sessionSummary": "记录了一条 Rust 冒烟测试任务",
                "tasks": [],
                "habits": [],
                "toolCalls": []
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

        let result = orchestrator
            .process_message("帮我记一下明天上午十点写 Rust 冒烟测试".to_string())
            .await
            .unwrap();

        assert_eq!(result.tasks.as_ref().map(|tasks| tasks.len()), Some(1));
        assert_eq!(
            result.new_memories,
            Some(vec!["用户最近在推进 Rust 测试相关开发".to_string()])
        );
        assert_eq!(
            result.user_profile.as_deref(),
            Some("用户当前在进行 Rust 工程开发")
        );
        assert_eq!(
            result.session_summary.as_deref(),
            Some("记录了一条 Rust 冒烟测试任务")
        );
    }
}

use async_trait::async_trait;
use futures::future::join_all;
use std::sync::Arc;
use crate::domain::models::{MixedResult, AIError, ChatRequest, ChatMessage, AgentRole};
use crate::domain::traits::{LlmGateway, AgentOrchestrator, ToolExecutor, MemoryStore};
use crate::application::prompt_builder::PromptBuilder;

pub struct Orchestrator {
    llm_gateway: Arc<dyn LlmGateway>,
    memory_store: Arc<dyn MemoryStore>,
    pub preferred_lang: String,
}

impl Orchestrator {
    pub fn new(llm_gateway: Arc<dyn LlmGateway>, memory_store: Arc<dyn MemoryStore>, preferred_lang: String) -> Self {
        Self {
            llm_gateway,
            memory_store,
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

    /// 第一阶段：Supervisor 路由
    async fn route(&self, text: &str) -> Result<MixedResult, AIError> {
        let system_prompt = PromptBuilder::build_supervisor_prompt(&self.preferred_lang);
        let messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt },
            ChatMessage { role: "user".to_string(), content: text.to_string() },
        ];

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
        let system_prompt = match role {
            AgentRole::TaskAgent => PromptBuilder::build_task_agent_prompt(&self.preferred_lang),
            AgentRole::HabitAgent => PromptBuilder::build_habit_agent_prompt(&self.preferred_lang),
            _ => "你是 Deadliner AI 助手。".to_string(),
        };

        let messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt },
            ChatMessage { role: "user".to_string(), content: text.to_string() },
        ];

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
        let worker_result: MixedResult = serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;
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
            if let Ok(worker_res) = res {
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
            }
        }

        Ok(final_result)
    }
}

#[async_trait]
impl ToolExecutor for Orchestrator {
    async fn submit_tool_result(&self, _tool_call_id: String, _result_json: String) -> Result<MixedResult, AIError> {
        Err(AIError::ParsingFailed)
    }
}

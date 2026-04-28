use super::*;
use std::collections::VecDeque;

use serde_json::json;

use crate::domain::models::{
    ChatChoice, ChatResponse, MemoryFragment, MemorySnapshot, MemorySyncPayload, ToolCapability,
    ToolMetadata,
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

    fn get_temperature(&self) -> f64 {
        0.1
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

    async fn append_conversation_turn(
        &self,
        _role: String,
        _content: String,
    ) -> Result<(), AIError> {
        Ok(())
    }

    async fn get_recent_conversation_context(&self, _limit: usize) -> Result<String, AIError> {
        Ok(String::new())
    }
}

fn build_test_registry() -> Arc<SkillRegistry> {
    Arc::new(SkillRegistry::from_skills(vec![
        ToolMetadata {
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
        },
        ToolMetadata {
            name: "create_task".to_string(),
            description: "创建任务".to_string(),
            domain: AgentRole::TaskAgent,
            capabilities: vec![ToolCapability::CreateTask],
            supported_platforms: vec!["ios".to_string(), "android".to_string()],
            requires_ui_confirmation: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "dueTime": { "type": "string" },
                    "note": { "type": "string" }
                },
                "required": ["name"]
            }),
        },
    ]))
}

include!("flow_loop_tests.rs");
include!("normalization_tests.rs");
include!("query_observation_tests.rs");
include!("memory_tests.rs");

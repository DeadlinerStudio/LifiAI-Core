use std::sync::Arc;
use tokio::sync::Mutex;
use std::path::PathBuf;
use crate::domain::traits::{AgentOrchestrator, MemoryStore, ToolExecutor};
use crate::infrastructure::llm_client::DeepSeekClient;
use crate::infrastructure::memory_store::LocalMemoryStore;
use crate::infrastructure::skill_registry::SkillRegistry;
use crate::application::orchestrator::Orchestrator;
pub use crate::interfaces::events::{CoreEvent, FFITask, FFIHabit};
use crate::domain::models::MixedResult;

// MARK: - Callback Trait

pub trait CoreCallback: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

// MARK: - Core Implementation

pub struct DeadlinerCore {
    orchestrator: Arc<Orchestrator>,
    memory_store: Arc<LocalMemoryStore>,
    callback: Arc<Mutex<Option<Box<dyn CoreCallback>>>>,
}

impl DeadlinerCore {
    pub fn new(
        api_key: String,
        base_url: String,
        model_id: String,
        storage_path: String,
        platform: String,
    ) -> Arc<Self> {
        let storage_dir = PathBuf::from(storage_path);
        if !storage_dir.exists() {
            let _ = std::fs::create_dir_all(&storage_dir);
        }

        let memory_store = Arc::new(LocalMemoryStore::new(storage_dir));
        let llm_gateway = Arc::new(DeepSeekClient::new(api_key, base_url, model_id));
        let skill_registry = Arc::new(
            SkillRegistry::load_from_dir("./skills").unwrap_or_else(|_| SkillRegistry::empty())
        );
        let orchestrator = Arc::new(Orchestrator::new(
            llm_gateway, 
            memory_store.clone() as Arc<dyn MemoryStore>, 
            skill_registry,
            platform,
            "zh-CN".to_string()
        ));

        Arc::new(Self {
            orchestrator,
            memory_store,
            callback: Arc::new(Mutex::new(None)),
        })
    }

    pub fn set_callback(&self, callback: Box<dyn CoreCallback>) {
        let mut cb = self.callback.blocking_lock();
        *cb = Some(callback);
    }

    pub async fn process_input(&self, text: String) {
        self.send_event(CoreEvent::OnThinking { agent_name: "Supervisor".to_string() }).await;

        match self.orchestrator.process_message(text.clone()).await {
            Ok(result) => self.handle_mixed_result(text, result).await,
            Err(e) => self.send_event(CoreEvent::OnError { message: e.to_string() }).await,
        }
    }

    pub async fn submit_tool_result(&self, id: String, result_json: String) {
        match self.orchestrator.submit_tool_result(id, result_json).await {
            Ok(result) => self.handle_mixed_result(String::new(), result).await,
            Err(e) => self.send_event(CoreEvent::OnError { message: e.to_string() }).await,
        }
    }

    pub fn get_latest_tasks(&self) -> Vec<FFITask> {
        vec![]
    }

    pub fn get_user_profile(&self) -> String {
        tokio::runtime::Handle::current().block_on(async {
            self.memory_store.get_user_profile().await.unwrap_or_default()
        })
    }

    async fn handle_mixed_result(&self, original_text: String, result: MixedResult) {
        if let Some(tool_calls) = result.tool_calls {
            if let Some(tool) = tool_calls.first() {
                let tool_call_id = uuid::Uuid::new_v4().to_string();
                if let Some(agent) = tool.agent.clone() {
                    self.orchestrator
                        .register_pending_tool_call(tool_call_id.clone(), agent, original_text)
                        .await;
                }
                self.send_event(CoreEvent::OnToolRequest { 
                    id: tool_call_id,
                    tool_name: tool.tool.clone(),
                    args_json: tool.args.to_string(),
                }).await;
                return;
            }
        }

        let tasks = result.tasks.map(|ts| ts.into_iter().map(|t| FFITask {
            name: t.name,
            due_time: t.due_time,
            note: t.note,
        }).collect());

        let habits = result.habits.map(|hs| hs.into_iter().map(|h| FFIHabit {
            name: h.name,
            period: h.period,
            times_per_period: h.times_per_period,
            goal_type: h.goal_type,
            total_target: h.total_target,
        }).collect());

        self.send_event(CoreEvent::OnFinish { 
            primary_intent: result.primary_intent.unwrap_or_default(),
            tasks,
            habits,
            chat_response: result.chat_response,
            session_summary: result.session_summary,
        }).await;
    }

    async fn send_event(&self, event: CoreEvent) {
        let cb = self.callback.lock().await;
        if let Some(callback) = cb.as_ref() {
            callback.on_event(event);
        }
    }
}

pub fn extract_tasks(_text: String, _preferred_lang: String) -> Result<Vec<FFITask>, CoreError> {
    Ok(vec![])
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("NetworkError")]
    NetworkError,
    #[error("ParsingFailed")]
    ParsingFailed,
    #[error("EmptyResponse")]
    EmptyResponse,
    #[error("MissingAPIKey")]
    MissingAPIKey,
}

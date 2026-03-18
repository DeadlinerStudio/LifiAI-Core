use std::sync::Arc;
use tokio::sync::Mutex;
use std::path::PathBuf;
use crate::domain::traits::{AgentOrchestrator, MemoryStore};
use crate::infrastructure::llm_client::DeepSeekClient;
use crate::infrastructure::memory_store::LocalMemoryStore;
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
    pub fn new(api_key: String, base_url: String, model_id: String, storage_path: String) -> Arc<Self> {
        let storage_dir = PathBuf::from(storage_path);
        if !storage_dir.exists() {
            let _ = std::fs::create_dir_all(&storage_dir);
        }

        let memory_store = Arc::new(LocalMemoryStore::new(storage_dir));
        let llm_gateway = Arc::new(DeepSeekClient::new(api_key, base_url, model_id));
        let orchestrator = Arc::new(Orchestrator::new(
            llm_gateway, 
            memory_store.clone() as Arc<dyn MemoryStore>, 
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

        match self.orchestrator.process_message(text).await {
            Ok(result) => self.handle_mixed_result(result).await,
            Err(e) => self.send_event(CoreEvent::OnError { message: e.to_string() }).await,
        }
    }

    pub async fn submit_tool_result(&self, _id: String, _result_json: String) {
        // 多 Agent 模式下的回灌逻辑
    }

    pub fn get_latest_tasks(&self) -> Vec<FFITask> {
        vec![]
    }

    pub fn get_user_profile(&self) -> String {
        tokio::runtime::Handle::current().block_on(async {
            self.memory_store.get_user_profile().await.unwrap_or_default()
        })
    }

    async fn handle_mixed_result(&self, result: MixedResult) {
        if let Some(tool_calls) = result.tool_calls {
            if let Some(tool) = tool_calls.first() {
                self.send_event(CoreEvent::OnToolRequest { 
                    id: uuid::Uuid::new_v4().to_string(), 
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

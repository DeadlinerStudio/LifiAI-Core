use std::sync::Arc;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tokio::runtime::Runtime;
use crate::domain::traits::MemoryStore;
use crate::infrastructure::llm_client::DeepSeekClient;
use crate::infrastructure::memory_store::LocalMemoryStore;
use crate::infrastructure::skill_registry::SkillRegistry;
use crate::application::orchestrator::{Orchestrator, OrchestratorProgressSink};
pub use crate::interfaces::events::{CoreEvent, FFITask, FFIHabit};
use crate::domain::models::{MemorySnapshot, MixedResult};

// MARK: - Callback Trait

pub trait CoreCallback: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

struct FfiProgressSink {
    callback: Arc<Mutex<Option<Box<dyn CoreCallback>>>>,
}

#[async_trait::async_trait]
impl OrchestratorProgressSink for FfiProgressSink {
    async fn on_thinking(&self, agent_name: String, phase: String, message: Option<String>) {
        let cb = self.callback.lock().await;
        if let Some(callback) = cb.as_ref() {
            callback.on_event(CoreEvent::OnThinking {
                agent_name,
                phase,
                message,
            });
        }
    }
}

#[derive(Debug, Clone, Default)]
struct LatestState {
    tasks: Vec<FFITask>,
    habits: Vec<FFIHabit>,
    memory_sync_json: Option<String>,
    finish_json: Option<String>,
}

// MARK: - Core Implementation

pub struct DeadlinerCore {
    orchestrator: Arc<Orchestrator>,
    memory_store: Arc<LocalMemoryStore>,
    runtime: Arc<Runtime>,
    callback: Arc<Mutex<Option<Box<dyn CoreCallback>>>>,
    latest_state: Arc<Mutex<LatestState>>,
}

impl DeadlinerCore {
    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
    }

    pub fn new(
        api_key: String,
        base_url: String,
        model_id: String,
        storage_path: String,
        platform: String,
    ) -> Arc<Self> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build Tokio runtime for DeadlinerCore"),
        );

        let storage_dir = PathBuf::from(storage_path);
        if !storage_dir.exists() {
            let _ = std::fs::create_dir_all(&storage_dir);
        }

        let memory_store = Arc::new(LocalMemoryStore::new(storage_dir));
        let llm_gateway = Arc::new(DeepSeekClient::new(api_key, base_url, model_id));
        let callback = Arc::new(Mutex::new(None));
        let skills_dir = Self::workspace_root().join("skills");
        let skill_registry = Arc::new(
            SkillRegistry::load_from_dir(skills_dir).unwrap_or_else(|_| SkillRegistry::empty())
        );
        let orchestrator = Arc::new(Orchestrator::new(
            llm_gateway, 
            memory_store.clone() as Arc<dyn MemoryStore>, 
            skill_registry,
            platform,
            "zh-CN".to_string()
        ));
        runtime.block_on(orchestrator.set_progress_sink(Arc::new(FfiProgressSink {
            callback: callback.clone(),
        })));

        Arc::new(Self {
            orchestrator,
            memory_store,
            runtime,
            callback,
            latest_state: Arc::new(Mutex::new(LatestState::default())),
        })
    }

    pub fn set_callback(&self, callback: Box<dyn CoreCallback>) {
        let mut cb = self.callback.blocking_lock();
        *cb = Some(callback);
    }

    pub async fn process_input(&self, text: String) {
        let orchestrator = self.orchestrator.clone();
        let runtime = self.runtime.clone();
        let this = self.clone_for_spawn();

        runtime.spawn(async move {
            match orchestrator.process_message_core(text.clone()).await {
                Ok(result) => {
                    let should_commit_memory =
                        !matches!(result.tool_calls.as_ref(), Some(tool_calls) if !tool_calls.is_empty());
                    let result_for_memory = result.clone();

                    this.handle_mixed_result(text.clone(), result).await;

                    if should_commit_memory {
                        this.commit_memory_after_finish(text, result_for_memory).await;
                    }
                }
                Err(e) => this.send_event(CoreEvent::OnError { message: e.to_string() }).await,
            }
        });
    }

    pub async fn submit_tool_result(&self, id: String, result_json: String) {
        let orchestrator = self.orchestrator.clone();
        let runtime = self.runtime.clone();
        let this = self.clone_for_spawn();

        runtime.spawn(async move {
            match orchestrator.submit_tool_result_core(id, result_json).await {
                Ok((original_text, result)) => {
                    let should_commit_memory =
                        !matches!(result.tool_calls.as_ref(), Some(tool_calls) if !tool_calls.is_empty());
                    let result_for_memory = result.clone();

                    this.handle_mixed_result(original_text.clone(), result).await;

                    if should_commit_memory {
                        this.commit_memory_after_finish(original_text, result_for_memory).await;
                    }
                }
                Err(e) => this.send_event(CoreEvent::OnError { message: e.to_string() }).await,
            }
        });
    }

    pub fn get_latest_tasks(&self) -> Vec<FFITask> {
        self.latest_state.blocking_lock().tasks.clone()
    }

    pub fn get_latest_habits(&self) -> Vec<FFIHabit> {
        self.latest_state.blocking_lock().habits.clone()
    }

    pub fn get_last_memory_sync_json(&self) -> Option<String> {
        self.latest_state.blocking_lock().memory_sync_json.clone()
    }

    pub fn get_last_finish_json(&self) -> Option<String> {
        self.latest_state.blocking_lock().finish_json.clone()
    }

    pub fn get_user_profile(&self) -> String {
        self.runtime.block_on(async {
            self.memory_store.get_user_profile().await.unwrap_or_default()
        })
    }

    pub async fn replace_memory_snapshot(&self, snapshot_json: String) {
        let memory_store = self.memory_store.clone();
        let runtime = self.runtime.clone();
        let this = self.clone_for_spawn();

        runtime.spawn(async move {
            let result = match serde_json::from_str::<MemorySnapshot>(&snapshot_json) {
                Ok(snapshot) => memory_store.replace_snapshot(snapshot).await.map_err(|_| CoreError::ParsingFailed),
                Err(_) => Err(CoreError::ParsingFailed),
            };

            if let Err(err) = result {
                this.send_event(CoreEvent::OnError {
                    message: err.to_string(),
                })
                .await;
            }
        });
    }

    pub fn export_memory_snapshot(&self) -> String {
        self.runtime.block_on(async {
            self.memory_store
                .current_snapshot()
                .await
                .ok()
                .and_then(|snapshot| serde_json::to_string(&snapshot).ok())
                .unwrap_or_else(|| "{\"revision\":0,\"fragments\":[],\"userProfile\":\"\"}".to_string())
        })
    }

    async fn handle_mixed_result(&self, original_text: String, result: MixedResult) {
        if let Some(tool_calls) = result.tool_calls {
            if let Some(tool) = tool_calls.first() {
                let tool_call_id = uuid::Uuid::new_v4().to_string();
                if let Some(agent) = tool.agent.clone() {
                    self.send_event(CoreEvent::OnThinking {
                        agent_name: agent.as_label().to_string(),
                        phase: "tool_wait".to_string(),
                        message: Some("正在等待工具执行结果".to_string()),
                    })
                    .await;
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

        let finish_json = serde_json::json!({
            "primaryIntent": result.primary_intent.clone().unwrap_or_default(),
            "tasks": tasks.clone(),
            "habits": habits.clone(),
            "chatResponse": result.chat_response.clone(),
            "sessionSummary": result.session_summary.clone(),
            "memorySyncJson": serde_json::Value::Null,
        })
        .to_string();

        {
            let mut latest = self.latest_state.lock().await;
            latest.tasks = tasks.clone().unwrap_or_default();
            latest.habits = habits.clone().unwrap_or_default();
            latest.memory_sync_json = None;
            latest.finish_json = Some(finish_json);
        }

        self.send_event(CoreEvent::OnFinish {
            primary_intent: result.primary_intent.unwrap_or_default(),
            tasks,
            habits,
            chat_response: result.chat_response,
            session_summary: result.session_summary,
            memory_sync_json: None,
        })
        .await;
    }

    async fn commit_memory_after_finish(&self, original_text: String, result: MixedResult) {
        match self
            .orchestrator
            .synthesize_and_commit_memory(original_text, result)
            .await
        {
            Ok(commit) => {
                {
                    let mut latest = self.latest_state.lock().await;
                    latest.memory_sync_json = commit.memory_sync_json.clone();
                }

                if commit.memory_sync_json.is_some() {
                    self.send_event(CoreEvent::OnMemoryCommitted {
                        added_memories: commit.added_memories,
                        profile_updated: commit.profile_updated,
                        new_revision: commit.new_revision,
                    })
                    .await;
                }
            }
            Err(e) => {
                self.send_event(CoreEvent::OnError {
                    message: e.to_string(),
                })
                .await;
            }
        }
    }

    async fn send_event(&self, event: CoreEvent) {
        let cb = self.callback.lock().await;
        if let Some(callback) = cb.as_ref() {
            callback.on_event(event);
        }
    }

    fn clone_for_spawn(&self) -> Self {
        Self {
            orchestrator: self.orchestrator.clone(),
            memory_store: self.memory_store.clone(),
            runtime: self.runtime.clone(),
            callback: self.callback.clone(),
            latest_state: self.latest_state.clone(),
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

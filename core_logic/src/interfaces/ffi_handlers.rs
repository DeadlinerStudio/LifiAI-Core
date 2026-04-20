use crate::application::orchestrator::{Orchestrator, OrchestratorProgressSink};
use crate::domain::models::{CapabilityExecutionMode, MemorySnapshot, MixedResult};
use crate::domain::traits::MemoryStore;
use crate::infrastructure::llm_client::DeepSeekClient;
use crate::infrastructure::memory_store::LocalMemoryStore;
use crate::infrastructure::skill_registry::SkillRegistry;
pub use crate::interfaces::events::{CoreEvent, FFIHabit, FFITask};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

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
    retrieved_tasks: Vec<FFITask>,
    retrieved_habits: Vec<FFIHabit>,
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
    ) -> Result<Arc<Self>, CoreError> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|err| {
                    CoreError::InitFailed(format!("failed to build Tokio runtime: {err}"))
                })?,
        );

        let storage_dir = PathBuf::from(storage_path);
        if !storage_dir.exists() {
            let _ = std::fs::create_dir_all(&storage_dir);
        }

        let memory_store = Arc::new(LocalMemoryStore::new(storage_dir));
        let llm_gateway = Arc::new(DeepSeekClient::new(api_key, base_url, model_id));
        let callback = Arc::new(Mutex::new(None));
        let skills_dir = Self::workspace_root().join("skills");
        let skill_registry = Arc::new(SkillRegistry::load_from_dir(skills_dir).map_err(|err| {
            CoreError::InitFailed(format!("failed to load skills from directory: {err}"))
        })?);
        let orchestrator = Arc::new(Orchestrator::new(
            llm_gateway,
            memory_store.clone() as Arc<dyn MemoryStore>,
            skill_registry,
            platform,
            "zh-CN".to_string(),
        ));
        runtime
            .block_on(orchestrator.set_progress_sink(Arc::new(FfiProgressSink {
                callback: callback.clone(),
            })))
            .map_err(|err| {
                CoreError::InitFailed(format!("failed to set progress sink: {err}"))
            })?;

        Ok(Arc::new(Self {
            orchestrator,
            memory_store,
            runtime,
            callback,
            latest_state: Arc::new(Mutex::new(LatestState::default())),
        }))
    }

    pub fn set_callback(&self, callback: Box<dyn CoreCallback>) {
        let mut cb = self.callback.blocking_lock();
        *cb = Some(callback);
    }

    pub async fn process_input(&self, text: String) {
        let orchestrator = self.orchestrator.clone();
        let runtime = self.runtime.clone();
        let this = self.clone_for_spawn();
        let request_id = uuid::Uuid::new_v4().to_string();

        runtime.spawn(async move {
            this.emit_lifecycle(
                request_id.clone(),
                "process_input",
                "started",
                Some("收到用户输入".to_string()),
            )
            .await;
            match orchestrator.process_message_core(text.clone()).await {
                Ok(result) => {
                    let should_commit_memory =
                        !matches!(result.tool_calls.as_ref(), Some(tool_calls) if !tool_calls.is_empty());
                    let result_for_memory = result.clone();

                    this.handle_mixed_result(request_id.clone(), text.clone(), result)
                        .await;

                    if should_commit_memory {
                        this.commit_memory_after_finish(request_id.clone(), text, result_for_memory)
                            .await;
                    }
                }
                Err(e) => {
                    this.emit_lifecycle(
                        request_id.clone(),
                        "process_input",
                        "failed",
                        Some(e.to_string()),
                    )
                    .await;
                    this.send_event(CoreEvent::OnError { message: e.to_string() }).await;
                }
            }
        });
    }

    pub async fn submit_tool_result(&self, id: String, result_json: String) {
        let orchestrator = self.orchestrator.clone();
        let runtime = self.runtime.clone();
        let this = self.clone_for_spawn();
        let request_id = uuid::Uuid::new_v4().to_string();

        runtime.spawn(async move {
            this.emit_lifecycle(
                request_id.clone(),
                "submit_tool_result",
                "started",
                Some("收到工具回灌结果".to_string()),
            )
            .await;
            match orchestrator.submit_tool_result_core(id, result_json).await {
                Ok((original_text, result)) => {
                    let should_commit_memory =
                        !matches!(result.tool_calls.as_ref(), Some(tool_calls) if !tool_calls.is_empty());
                    let result_for_memory = result.clone();

                    this.handle_mixed_result(request_id.clone(), original_text.clone(), result)
                        .await;

                    if should_commit_memory {
                        this.commit_memory_after_finish(
                            request_id.clone(),
                            original_text,
                            result_for_memory,
                        )
                        .await;
                    }
                }
                Err(e) => {
                    this.emit_lifecycle(
                        request_id.clone(),
                        "submit_tool_result",
                        "failed",
                        Some(e.to_string()),
                    )
                    .await;
                    this.send_event(CoreEvent::OnError { message: e.to_string() }).await;
                }
            }
        });
    }

    pub fn get_latest_tasks(&self) -> Vec<FFITask> {
        self.latest_state.blocking_lock().tasks.clone()
    }

    pub fn get_latest_habits(&self) -> Vec<FFIHabit> {
        self.latest_state.blocking_lock().habits.clone()
    }

    pub fn get_latest_retrieved_tasks(&self) -> Vec<FFITask> {
        self.latest_state.blocking_lock().retrieved_tasks.clone()
    }

    pub fn get_latest_retrieved_habits(&self) -> Vec<FFIHabit> {
        self.latest_state.blocking_lock().retrieved_habits.clone()
    }

    pub fn get_last_memory_sync_json(&self) -> Option<String> {
        self.latest_state.blocking_lock().memory_sync_json.clone()
    }

    pub fn get_last_finish_json(&self) -> Option<String> {
        self.latest_state.blocking_lock().finish_json.clone()
    }

    pub fn get_user_profile(&self) -> String {
        self.runtime.block_on(async {
            self.memory_store
                .get_user_profile()
                .await
                .unwrap_or_default()
        })
    }

    pub async fn replace_memory_snapshot(&self, snapshot_json: String) {
        let memory_store = self.memory_store.clone();
        let runtime = self.runtime.clone();
        let this = self.clone_for_spawn();

        runtime.spawn(async move {
            let result = match serde_json::from_str::<MemorySnapshot>(&snapshot_json) {
                Ok(snapshot) => memory_store
                    .replace_snapshot(snapshot)
                    .await
                    .map_err(|_| CoreError::ParsingFailed),
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
                .unwrap_or_else(|| {
                    "{\"revision\":0,\"fragments\":[],\"userProfile\":\"\"}".to_string()
                })
        })
    }

    async fn handle_mixed_result(
        &self,
        request_id: String,
        original_text: String,
        result: MixedResult,
    ) {
        if let Some(tool_calls) = result.tool_calls {
            if let Some(tool) = tool_calls.first() {
                let tool_call_id = uuid::Uuid::new_v4().to_string();
                let retry_count = Self::extract_retry_count(&tool.args);
                if let Some(agent) = tool.agent.clone() {
                    self.send_event(CoreEvent::OnThinking {
                        agent_name: agent.as_label().to_string(),
                        phase: "tool_wait".to_string(),
                        message: Some("正在等待工具执行结果".to_string()),
                    })
                    .await;
                    self.orchestrator
                        .register_pending_tool_call(
                            tool_call_id.clone(),
                            agent,
                            original_text,
                            tool.tool.clone(),
                            retry_count,
                        )
                        .await;
                }
                self.emit_lifecycle(
                    request_id,
                    "tool_request",
                    "waiting",
                    Some(format!("等待工具 {} 执行", tool.tool)),
                )
                .await;
                self.send_event(CoreEvent::OnToolRequest {
                    id: tool_call_id,
                    tool_name: tool.tool.clone(),
                    args_json: Self::tool_request_args_json(
                        &tool.args,
                        tool.execution_mode.as_ref(),
                    ),
                })
                .await;
                return;
            }
        }

        let tasks = result.tasks.map(|ts| {
            ts.into_iter()
                .map(|t| FFITask {
                    name: t.name,
                    due_time: t.due_time,
                    note: t.note,
                })
                .collect()
        });

        let habits = result.habits.map(|hs| {
            hs.into_iter()
                .map(|h| FFIHabit {
                    name: h.name,
                    period: h.period,
                    times_per_period: h.times_per_period,
                    goal_type: h.goal_type,
                    total_target: h.total_target,
                })
                .collect()
        });

        let retrieved_tasks = result.retrieved_tasks.map(|ts| {
            ts.into_iter()
                .map(|t| FFITask {
                    name: t.name,
                    due_time: t.due_time,
                    note: t.note,
                })
                .collect()
        });

        let retrieved_habits = result.retrieved_habits.map(|hs| {
            hs.into_iter()
                .map(|h| FFIHabit {
                    name: h.name,
                    period: h.period,
                    times_per_period: h.times_per_period,
                    goal_type: h.goal_type,
                    total_target: h.total_target,
                })
                .collect()
        });

        let finish_json = serde_json::json!({
            "primaryIntent": result.primary_intent.clone().unwrap_or_default(),
            "tasks": tasks.clone(),
            "habits": habits.clone(),
            "retrievedTasks": retrieved_tasks.clone(),
            "retrievedHabits": retrieved_habits.clone(),
            "chatResponse": result.chat_response.clone(),
            "sessionSummary": result.session_summary.clone(),
            "memorySyncJson": serde_json::Value::Null,
        })
        .to_string();

        {
            let mut latest = self.latest_state.lock().await;
            latest.tasks = tasks.clone().unwrap_or_default();
            latest.habits = habits.clone().unwrap_or_default();
            latest.retrieved_tasks = retrieved_tasks.clone().unwrap_or_default();
            latest.retrieved_habits = retrieved_habits.clone().unwrap_or_default();
            latest.memory_sync_json = None;
            latest.finish_json = Some(finish_json);
        }

        self.send_event(CoreEvent::OnFinish {
            primary_intent: result.primary_intent.unwrap_or_default(),
            tasks,
            habits,
            retrieved_tasks,
            retrieved_habits,
            chat_response: result.chat_response,
            session_summary: result.session_summary,
            memory_sync_json: None,
        })
        .await;
        self.emit_lifecycle(
            request_id,
            "orchestration",
            "completed",
            Some("主流程完成并回传结果".to_string()),
        )
        .await;
    }

    async fn commit_memory_after_finish(
        &self,
        request_id: String,
        original_text: String,
        result: MixedResult,
    ) {
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
                    self.emit_lifecycle(
                        request_id,
                        "memory_commit",
                        "completed",
                        Some("记忆增量已提交".to_string()),
                    )
                    .await;
                }
            }
            Err(e) => {
                self.emit_lifecycle(request_id, "memory_commit", "failed", Some(e.to_string()))
                    .await;
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

    async fn emit_lifecycle(
        &self,
        request_id: String,
        stage: &str,
        status: &str,
        message: Option<String>,
    ) {
        self.send_event(CoreEvent::OnLifecycle {
            request_id,
            stage: stage.to_string(),
            status: status.to_string(),
            message,
        })
        .await;
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

    fn tool_request_args_json(
        args: &serde_json::Value,
        execution_mode: Option<&CapabilityExecutionMode>,
    ) -> String {
        let mut payload = args.clone();

        if let Some(mode) = execution_mode {
            if let Some(object) = payload.as_object_mut() {
                let mode_str = match mode {
                    CapabilityExecutionMode::Auto => "AUTO",
                    CapabilityExecutionMode::AskUser => "ASK_USER",
                    CapabilityExecutionMode::Never => "NEVER",
                };
                let meta_entry = object
                    .entry("_meta".to_string())
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(meta_object) = meta_entry.as_object_mut() {
                    meta_object.insert("executionMode".to_string(), serde_json::json!(mode_str));
                }
            }
        }

        payload.to_string()
    }

    fn extract_retry_count(args: &serde_json::Value) -> u8 {
        args.get("_meta")
            .and_then(|meta| meta.get("retryCount"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .min(u64::from(u8::MAX)) as u8
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
    #[error("InitFailed: {0}")]
    InitFailed(String),
}

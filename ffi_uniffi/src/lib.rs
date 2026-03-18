use std::sync::Arc;

use core_logic::interfaces::events as inner_events;
use core_logic::interfaces::ffi_handlers as inner_handlers;

uniffi::setup_scaffolding!();

#[derive(Debug, Clone, uniffi::Record)]
pub struct FFITask {
    pub name: String,
    pub due_time: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FFIHabit {
    pub name: String,
    pub period: String,
    pub times_per_period: i32,
    pub goal_type: String,
    pub total_target: Option<i32>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum CoreEvent {
    OnThinking { agent_name: String },
    OnTextStream { chunk: String },
    OnToolRequest { id: String, tool_name: String, args_json: String },
    OnFinish {
        primary_intent: String,
        tasks: Option<Vec<FFITask>>,
        habits: Option<Vec<FFIHabit>>,
        chat_response: Option<String>,
        session_summary: Option<String>,
        memory_sync_json: Option<String>,
    },
    OnError { message: String },
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
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

#[uniffi::export(callback_interface)]
pub trait CoreCallback: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

struct CallbackAdapter {
    inner: Box<dyn CoreCallback>,
}

impl inner_handlers::CoreCallback for CallbackAdapter {
    fn on_event(&self, event: inner_events::CoreEvent) {
        self.inner.on_event(event.into());
    }
}

#[derive(uniffi::Object)]
pub struct DeadlinerCore {
    inner: Arc<inner_handlers::DeadlinerCore>,
}

#[uniffi::export]
impl DeadlinerCore {
    #[uniffi::constructor]
    pub fn new(
        api_key: String,
        base_url: String,
        model_id: String,
        storage_path: String,
        platform: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: inner_handlers::DeadlinerCore::new(
                api_key,
                base_url,
                model_id,
                storage_path,
                platform,
            ),
        })
    }

    pub fn set_callback(&self, callback: Box<dyn CoreCallback>) {
        self.inner
            .set_callback(Box::new(CallbackAdapter { inner: callback }));
    }

    pub async fn process_input(&self, text: String) {
        self.inner.process_input(text).await;
    }

    pub async fn submit_tool_result(&self, id: String, result_json: String) {
        self.inner.submit_tool_result(id, result_json).await;
    }

    pub fn get_latest_tasks(&self) -> Vec<FFITask> {
        self.inner
            .get_latest_tasks()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    pub fn get_latest_habits(&self) -> Vec<FFIHabit> {
        self.inner
            .get_latest_habits()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    pub fn get_last_memory_sync_json(&self) -> Option<String> {
        self.inner.get_last_memory_sync_json()
    }

    pub fn get_last_finish_json(&self) -> Option<String> {
        self.inner.get_last_finish_json()
    }

    pub fn get_user_profile(&self) -> String {
        self.inner.get_user_profile()
    }

    pub async fn replace_memory_snapshot(&self, snapshot_json: String) {
        self.inner.replace_memory_snapshot(snapshot_json).await;
    }

    pub fn export_memory_snapshot(&self) -> String {
        self.inner.export_memory_snapshot()
    }
}

#[uniffi::export]
pub fn extract_tasks(text: String, preferred_lang: String) -> Result<Vec<FFITask>, CoreError> {
    inner_handlers::extract_tasks(text, preferred_lang)
        .map(|tasks| tasks.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

impl From<inner_events::FFITask> for FFITask {
    fn from(value: inner_events::FFITask) -> Self {
        Self {
            name: value.name,
            due_time: value.due_time,
            note: value.note,
        }
    }
}

impl From<inner_events::FFIHabit> for FFIHabit {
    fn from(value: inner_events::FFIHabit) -> Self {
        Self {
            name: value.name,
            period: value.period,
            times_per_period: value.times_per_period,
            goal_type: value.goal_type,
            total_target: value.total_target,
        }
    }
}

impl From<inner_events::CoreEvent> for CoreEvent {
    fn from(value: inner_events::CoreEvent) -> Self {
        match value {
            inner_events::CoreEvent::OnThinking { agent_name } => Self::OnThinking { agent_name },
            inner_events::CoreEvent::OnTextStream { chunk } => Self::OnTextStream { chunk },
            inner_events::CoreEvent::OnToolRequest {
                id,
                tool_name,
                args_json,
            } => Self::OnToolRequest {
                id,
                tool_name,
                args_json,
            },
            inner_events::CoreEvent::OnFinish {
                primary_intent,
                tasks,
                habits,
                chat_response,
                session_summary,
                memory_sync_json,
            } => Self::OnFinish {
                primary_intent,
                tasks: tasks.map(|items| items.into_iter().map(Into::into).collect()),
                habits: habits.map(|items| items.into_iter().map(Into::into).collect()),
                chat_response,
                session_summary,
                memory_sync_json,
            },
            inner_events::CoreEvent::OnError { message } => Self::OnError { message },
        }
    }
}

impl From<inner_handlers::CoreError> for CoreError {
    fn from(value: inner_handlers::CoreError) -> Self {
        match value {
            inner_handlers::CoreError::NetworkError => Self::NetworkError,
            inner_handlers::CoreError::ParsingFailed => Self::ParsingFailed,
            inner_handlers::CoreError::EmptyResponse => Self::EmptyResponse,
            inner_handlers::CoreError::MissingAPIKey => Self::MissingAPIKey,
        }
    }
}

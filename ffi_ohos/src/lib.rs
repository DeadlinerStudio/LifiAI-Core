use std::sync::Arc;
use std::sync::Mutex;

use core_logic::interfaces::events::{CoreEvent, FFIHabit, FFITask};
use core_logic::interfaces::ffi_handlers::{CoreCallback, DeadlinerCore, extract_tasks};
use napi_derive_ohos::napi;
use serde_json::{Value, json};

use napi_ohos::bindgen_prelude::*;

fn task_to_json(task: &FFITask) -> Value {
    json!({
        "name": task.name,
        "dueTime": task.due_time,
        "note": task.note,
    })
}

fn habit_to_json(habit: &FFIHabit) -> Value {
    json!({
        "name": habit.name,
        "period": habit.period,
        "timesPerPeriod": habit.times_per_period,
        "goalType": habit.goal_type,
        "totalTarget": habit.total_target,
    })
}

fn event_to_json(event: CoreEvent) -> String {
    let payload = match event {
        CoreEvent::OnThinking {
            agent_name,
            phase,
            message,
        } => json!({
            "type": "thinking",
            "agentName": agent_name,
            "phase": phase,
            "message": message,
        }),
        CoreEvent::OnTextStream { chunk } => json!({
            "type": "textStream",
            "chunk": chunk,
        }),
        CoreEvent::OnToolRequest {
            id,
            tool_name,
            args_json,
        } => json!({
            "type": "toolRequest",
            "id": id,
            "toolName": tool_name,
            "argsJson": args_json,
        }),
        CoreEvent::OnFinish {
            primary_intent,
            tasks,
            habits,
            retrieved_tasks,
            retrieved_habits,
            chat_response,
            session_summary,
            memory_sync_json,
        } => json!({
            "type": "finish",
            "primaryIntent": primary_intent,
            "tasks": tasks.map(|items| items.iter().map(task_to_json).collect::<Vec<_>>()),
            "habits": habits.map(|items| items.iter().map(habit_to_json).collect::<Vec<_>>()),
            "retrievedTasks": retrieved_tasks.map(|items| items.iter().map(task_to_json).collect::<Vec<_>>()),
            "retrievedHabits": retrieved_habits.map(|items| items.iter().map(habit_to_json).collect::<Vec<_>>()),
            "chatResponse": chat_response,
            "sessionSummary": session_summary,
            "memorySyncJson": memory_sync_json,
        }),
        CoreEvent::OnMemoryCommitted {
            added_memories,
            profile_updated,
            new_revision,
        } => json!({
            "type": "memoryCommitted",
            "addedMemories": added_memories,
            "profileUpdated": profile_updated,
            "newRevision": new_revision,
        }),
        CoreEvent::OnError { message } => json!({
            "type": "error",
            "message": message,
        }),
    };

    payload.to_string()
}

fn stringify_tasks(tasks: Vec<FFITask>) -> Result<String> {
    serde_json::to_string(&tasks.iter().map(task_to_json).collect::<Vec<_>>())
        .map_err(|err| Error::from_reason(format!("Failed to serialize tasks: {err}")))
}

fn stringify_habits(habits: Vec<FFIHabit>) -> Result<String> {
    serde_json::to_string(&habits.iter().map(habit_to_json).collect::<Vec<_>>())
        .map_err(|err| Error::from_reason(format!("Failed to serialize habits: {err}")))
}

struct QueueingCallback {
    queue: Arc<Mutex<Vec<String>>>,
}

impl CoreCallback for QueueingCallback {
    fn on_event(&self, event: CoreEvent) {
        let payload = event_to_json(event);
        if let Ok(mut queue) = self.queue.lock() {
            queue.push(payload);
        }
    }
}

#[napi(js_name = "DeadlinerCore")]
pub struct DeadlinerCoreOhos {
    inner: Arc<DeadlinerCore>,
    event_queue: Arc<Mutex<Vec<String>>>,
}

#[napi]
impl DeadlinerCoreOhos {
    #[napi(constructor)]
    pub fn new(
        api_key: String,
        base_url: String,
        model_id: String,
        storage_path: String,
        platform: String,
    ) -> Self {
        let inner = DeadlinerCore::new(api_key, base_url, model_id, storage_path, platform);
        let event_queue = Arc::new(Mutex::new(Vec::new()));
        inner.set_callback(Box::new(QueueingCallback {
            queue: event_queue.clone(),
        }));

        Self { inner, event_queue }
    }

    #[napi]
    pub async fn process_input(&self, text: String) {
        self.inner.process_input(text).await;
    }

    #[napi]
    pub async fn submit_tool_result(&self, id: String, result_json: String) {
        self.inner.submit_tool_result(id, result_json).await;
    }

    #[napi]
    pub fn get_latest_tasks_json(&self) -> Result<String> {
        stringify_tasks(self.inner.get_latest_tasks())
    }

    #[napi]
    pub fn get_latest_habits_json(&self) -> Result<String> {
        stringify_habits(self.inner.get_latest_habits())
    }

    #[napi]
    pub fn get_last_memory_sync_json(&self) -> Option<String> {
        self.inner.get_last_memory_sync_json()
    }

    #[napi]
    pub fn get_last_finish_json(&self) -> Option<String> {
        self.inner.get_last_finish_json()
    }

    #[napi]
    pub fn drain_events_json(&self) -> Result<String> {
        let mut queue = self
            .event_queue
            .lock()
            .map_err(|_| Error::from_reason("Failed to lock event queue"))?;
        let events = std::mem::take(&mut *queue);
        serde_json::to_string(&events)
            .map_err(|err| Error::from_reason(format!("Failed to serialize events: {err}")))
    }

    #[napi]
    pub fn pending_event_count(&self) -> u32 {
        self.event_queue
            .lock()
            .map(|queue| queue.len() as u32)
            .unwrap_or(0)
    }

    #[napi]
    pub fn get_user_profile(&self) -> String {
        self.inner.get_user_profile()
    }

    #[napi]
    pub async fn replace_memory_snapshot(&self, snapshot_json: String) {
        self.inner.replace_memory_snapshot(snapshot_json).await;
    }

    #[napi]
    pub fn export_memory_snapshot(&self) -> String {
        self.inner.export_memory_snapshot()
    }
}

#[napi]
pub fn extract_tasks_json(text: String, preferred_lang: String) -> Result<String> {
    let tasks = extract_tasks(text, preferred_lang)
        .map_err(|err| Error::from_reason(format!("extract_tasks failed: {err}")))?;
    stringify_tasks(tasks)
}

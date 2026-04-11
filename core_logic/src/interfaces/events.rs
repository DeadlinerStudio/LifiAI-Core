use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum CoreEvent {
    OnLifecycle {
        request_id: String,
        stage: String,
        status: String,
        message: Option<String>,
    },
    OnThinking {
        agent_name: String,
        phase: String,
        message: Option<String>,
    },
    OnTextStream {
        chunk: String,
    },
    OnToolRequest {
        id: String,
        tool_name: String,
        args_json: String,
    },
    OnFinish {
        primary_intent: String,
        tasks: Option<Vec<FFITask>>,
        habits: Option<Vec<FFIHabit>>,
        retrieved_tasks: Option<Vec<FFITask>>,
        retrieved_habits: Option<Vec<FFIHabit>>,
        chat_response: Option<String>,
        session_summary: Option<String>,
        memory_sync_json: Option<String>,
    },
    OnMemoryCommitted {
        added_memories: Vec<String>,
        profile_updated: bool,
        new_revision: u64,
    },
    OnError {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FFITask {
    pub name: String,
    pub due_time: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FFIHabit {
    pub name: String,
    pub period: String,
    pub times_per_period: i32,
    pub goal_type: String,
    pub total_target: Option<i32>,
}

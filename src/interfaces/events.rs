use uniffi::Enum;
use crate::domain::models::{AITask, AIHabit, AIToolCall, MemoryFragment};

#[derive(Enum)]
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
    },
    OnError { message: String },
}

// UniFFI 不支持直接导出包含复杂的第三方库（如 serde_json::Value）的结构体
// 所以我们需要在接口层定义平铺的（Flat）DTO
#[derive(uniffi::Record)]
pub struct FFITask {
    pub name: String,
    pub due_time: Option<String>,
    pub note: Option<String>,
}

#[derive(uniffi::Record)]
pub struct FFIHabit {
    pub name: String,
    pub period: String,
    pub times_per_period: i32,
    pub goal_type: String,
    pub total_target: Option<i32>,
}

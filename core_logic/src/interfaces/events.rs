#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct FFITask {
    pub name: String,
    pub due_time: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FFIHabit {
    pub name: String,
    pub period: String,
    pub times_per_period: i32,
    pub goal_type: String,
    pub total_target: Option<i32>,
}

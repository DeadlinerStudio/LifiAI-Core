use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// 全局统一时间格式：YYYY-MM-DD HH:mm (本地时间)
pub const DEFAULT_TIME_FORMAT: &str = "%Y-%m-%d %H:%M";

// 内存管理配置
pub const MAX_MEMORY_FRAGMENTS: usize = 60;
pub const MAX_MEMORY_AGE_DAYS: i64 = 120;
pub const MAX_CONVERSATION_TURNS: usize = 24;
pub const MAX_CONVERSATION_CONTEXT_TURNS: usize = 8;
pub const MAX_CONVERSATION_MESSAGE_CHARS: usize = 400;

// MARK: - Multi-Agent & Tool Metadata (Section 4 & 5)

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum AgentRole {
    Supervisor,
    TaskAgent,
    HabitAgent,
    ChatAgent,
}

impl AgentRole {
    pub fn as_label(&self) -> &'static str {
        match self {
            AgentRole::Supervisor => "Supervisor",
            AgentRole::TaskAgent => "TaskAgent",
            AgentRole::HabitAgent => "HabitAgent",
            AgentRole::ChatAgent => "ChatAgent",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub domain: AgentRole,
    #[serde(default)]
    pub capabilities: Vec<ToolCapability>,
    pub supported_platforms: Vec<String>,
    pub requires_ui_confirmation: bool,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolCapability {
    ReadTaskContext,
    ReadHabitContext,
    CreateTask,
    CreateHabit,
    UpdateTaskDeadline,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CapabilityExecutionMode {
    Auto,
    AskUser,
    Never,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedToolCandidate {
    pub capability: ToolCapability,
    pub tool: ToolMetadata,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionPlan {
    pub capability: ToolCapability,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub requires_ui_confirmation: bool,
    pub execution_mode: CapabilityExecutionMode,
}

// MARK: - AI 提取结果模型

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFragment {
    pub id: Uuid,
    pub content: String,
    pub category: String,
    pub timestamp: DateTime<Utc>,
    pub importance: i32,
}

impl MemoryFragment {
    pub fn new(content: String, category: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            content,
            category,
            timestamp: Utc::now(),
            importance: 3,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

impl ConversationTurn {
    pub fn new(role: String, content: String) -> Self {
        Self {
            role,
            content,
            timestamp: Utc::now(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    pub revision: u64,
    pub fragments: Vec<MemoryFragment>,
    pub user_profile: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum MemorySyncOperation {
    UpsertFragment { fragment: MemoryFragment },
    DeleteFragment { fragment_id: Uuid },
    ReplaceUserProfile { profile: String },
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemorySyncPayload {
    pub base_revision: u64,
    pub next_revision: u64,
    pub operations: Vec<MemorySyncOperation>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AITask {
    pub name: String,
    /// 截止时间，格式固定为 "YYYY-MM-DD HH:mm"
    pub due_time: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AIHabit {
    pub name: String,
    pub period: String,
    pub times_per_period: i32,
    pub goal_type: String,
    pub total_target: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MixedResult {
    pub primary_intent: Option<String>,
    pub routed_agents: Option<Vec<AgentRole>>,
    pub tasks: Option<Vec<AITask>>,
    pub habits: Option<Vec<AIHabit>>,
    pub retrieved_tasks: Option<Vec<AITask>>,
    pub retrieved_habits: Option<Vec<AIHabit>>,
    pub new_memories: Option<Vec<String>>,
    pub chat_response: Option<String>,
    pub session_summary: Option<String>,
    pub user_profile: Option<String>,
    pub requested_capabilities: Option<Vec<CapabilityRequest>>,
    pub actions: Option<Vec<AIActionCall>>,
    pub tool_calls: Option<Vec<AIToolCall>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlannerResult {
    pub primary_intent: Option<String>,
    pub routed_agents: Option<Vec<AgentRole>>,
    pub tasks: Option<Vec<AITask>>,
    pub habits: Option<Vec<AIHabit>>,
    pub chat_response: Option<String>,
    pub new_memories: Option<Vec<String>>,
    pub session_summary: Option<String>,
    pub user_profile: Option<String>,
    pub requested_capabilities: Option<Vec<CapabilityRequest>>,
    pub actions: Option<Vec<AIActionCall>>,
    pub tool_calls: Option<Vec<AIToolCall>>,
}

impl From<PlannerResult> for MixedResult {
    fn from(value: PlannerResult) -> Self {
        Self {
            primary_intent: value.primary_intent,
            routed_agents: value.routed_agents,
            tasks: value.tasks,
            habits: value.habits,
            chat_response: value.chat_response,
            new_memories: value.new_memories,
            session_summary: value.session_summary,
            user_profile: value.user_profile,
            requested_capabilities: value.requested_capabilities,
            actions: value.actions,
            tool_calls: value.tool_calls,
            ..Default::default()
        }
    }
}

// MARK: - API 请求/响应模型

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatChoice {
    pub message: ChatMessage,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatResponse {
    pub choices: Vec<ChatChoice>,
}

// MARK: - 错误定义

#[derive(Debug, Clone)]
pub enum AIError {
    MissingAPIKey,
    EmptyResponse,
    ParsingFailed,
    NetworkError(String),
    ConfigError(String),
    InvalidToolCall(String),
}

impl fmt::Display for AIError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AIError::MissingAPIKey => write!(f, "请先在设置中填写 DeepSeek API Key"),
            AIError::EmptyResponse => write!(f, "AI 返回内容为空"),
            AIError::ParsingFailed => write!(f, "AI 数据解析失败"),
            AIError::NetworkError(err) => write!(f, "网络请求失败: {}", err),
            AIError::ConfigError(err) => write!(f, "配置错误: {}", err),
            AIError::InvalidToolCall(err) => write!(f, "工具调用无效: {}", err),
        }
    }
}

impl std::error::Error for AIError {}

// MARK: - Tool Calls

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AIActionCall {
    // Legacy compatibility path. New planner outputs should prefer requestedCapabilities.
    pub action: String,
    pub args: serde_json::Value,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRequest {
    pub capability: ToolCapability,
    pub args: serde_json::Value,
    pub reason: Option<String>,
    #[serde(default)]
    pub execution_mode: Option<CapabilityExecutionMode>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AIToolCall {
    pub agent: Option<AgentRole>,
    pub tool: String,
    pub args: serde_json::Value,
    pub reason: Option<String>,
    pub execution_mode: Option<CapabilityExecutionMode>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AIToolResult {
    pub id: Uuid,
    pub tool: String,
    pub applied_args: serde_json::Value,
    pub payload: serde_json::Value,
    pub generated_at: DateTime<Utc>,
}

impl AIToolResult {
    pub fn new(tool: String, applied_args: serde_json::Value, payload: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool,
            applied_args,
            payload,
            generated_at: Utc::now(),
        }
    }
}

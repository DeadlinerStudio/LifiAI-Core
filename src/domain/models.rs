use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// 全局统一时间格式：YYYY-MM-DD HH:mm (本地时间)
pub const DEFAULT_TIME_FORMAT: &str = "%Y-%m-%d %H:%M";

// 内存管理配置
pub const MAX_MEMORY_FRAGMENTS: usize = 60;
pub const MAX_MEMORY_AGE_DAYS: i64 = 120;

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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MixedResult {
    pub primary_intent: Option<String>,
    pub tasks: Option<Vec<AITask>>,
    pub habits: Option<Vec<AIHabit>>,
    pub new_memories: Option<Vec<String>>,
    pub chat_response: Option<String>,
    pub session_summary: Option<String>,
    pub user_profile: Option<String>,
    pub tool_calls: Option<Vec<AIToolCall>>,
}

// MARK: - API 请求/响应模型 (DeepSeek/OpenAI 兼容格式)
// 在严格的 DDD 中，外部 API 模型通常放在 infrastructure，但为了配合你的组织方式，暂时放在这里。

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
}

// 为 AIError 实现标准的 Display，这相当于 Swift 里的 LocalizedError
impl fmt::Display for AIError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AIError::MissingAPIKey => write!(f, "请先在设置中填写 DeepSeek API Key"),
            AIError::EmptyResponse => write!(f, "AI 返回内容为空"),
            AIError::ParsingFailed => write!(f, "AI 数据解析失败"),
            AIError::NetworkError(err) => write!(f, "网络请求失败: {}", err),
        }
    }
}

// 实现 Rust 标准库的 Error trait
impl std::error::Error for AIError {}

// MARK: - Tool Calls (MCP-like, local tools)

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AIToolCall {
    pub tool: String,
    pub args: serde_json::Value,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReadTasksArgs {
    pub time_range_days: Option<i32>,
    pub status: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub limit: Option<i32>,
    pub sort: Option<String>,
}

// MARK: - Tool Request / Result (App-side)

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AIToolRequest {
    pub id: Uuid,
    pub tool: String,
    pub args: serde_json::Value,
    pub reason: Option<String>,
}

impl AIToolRequest {
    pub fn new(tool: String, args: serde_json::Value, reason: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            tool,
            args,
            reason,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AIToolResult {
    pub id: Uuid,
    pub tool: String,
    pub applied_args: serde_json::Value,
    pub payload: serde_json::Value,
    pub generated_at: DateTime<Utc>, // 使用 UTC 时间，serde 会自动将其处理为 ISO8601 格式
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReadTasksResultPayload {
    pub tasks: Vec<TaskDigestItem>,
    pub summary: TaskSummary,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskDigestItem {
    pub id: i64,
    pub name: String,
    pub due: String,
    pub status: String,
    pub note_preview: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub count: i32,
    pub overdue: i32,
    pub due_soon_24h: i32,
}

// MARK: - Monthly Analysis

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyAnalysisResult {
    pub month: String,
    pub summary: String,
    pub keywords: Vec<String>,
    pub generated_at: Option<DateTime<Utc>>,
}

impl MonthlyAnalysisResult {
    pub fn new(month: String, summary: String, keywords: Vec<String>) -> Self {
        Self {
            month,
            summary,
            keywords,
            generated_at: Some(Utc::now()),
        }
    }
}
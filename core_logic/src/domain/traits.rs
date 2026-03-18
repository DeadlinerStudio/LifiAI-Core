use async_trait::async_trait;
use crate::domain::models::{ChatRequest, ChatResponse, AIError, MixedResult, MemoryFragment};

/// 大模型网关接口
/// 抽象了具体的 LLM 实现（如 DeepSeek, OpenAI）
#[async_trait]
pub trait LlmGateway: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, AIError>;
    fn get_model_name(&self) -> String;
}

/// Agent 编排器接口
/// 负责处理用户输入并协调多个 Worker Agent
#[async_trait]
pub trait AgentOrchestrator: Send + Sync {
    async fn process_message(&self, text: String) -> Result<MixedResult, AIError>;
}

/// 工具执行器接口
/// 在原生端执行完工具后，通过此接口回灌数据
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn submit_tool_result(&self, tool_call_id: String, result_json: String) -> Result<MixedResult, AIError>;
}

/// 内存存储接口 (MemoryBank 的 Rust 实现契约)
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// 保存新的记忆碎片
    async fn save_memory(&self, content: String, category: String) -> Result<(), AIError>;
    /// 获取所有记忆碎片
    async fn get_all_fragments(&self) -> Result<Vec<MemoryFragment>, AIError>;
    /// 更新用户画像
    async fn save_user_profile(&self, profile: String) -> Result<(), AIError>;
    /// 获取用户画像
    async fn get_user_profile(&self) -> Result<String, AIError>;
    /// 获取用于 LLM 提示词的长效上下文 (对应 Swift 的 getLongTermContext)
    async fn get_long_term_context(&self) -> Result<String, AIError>;
}

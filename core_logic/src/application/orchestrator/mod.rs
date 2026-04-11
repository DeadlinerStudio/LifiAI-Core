use crate::application::prompt_builder::PromptBuilder;
use crate::domain::models::{
    AIActionCall, AIError, AIToolCall, AgentRole, CapabilityExecutionMode, CapabilityRequest,
    ChatMessage, ChatRequest, MixedResult, PlannerResult, ToolCapability,
};
use crate::domain::traits::{AgentOrchestrator, LlmGateway, MemoryStore, ToolExecutor};
use crate::infrastructure::logger;
use crate::infrastructure::skill_registry::SkillRegistry;
use async_trait::async_trait;
use futures::future::join_all;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait OrchestratorProgressSink: Send + Sync {
    async fn on_thinking(&self, agent_name: String, phase: String, message: Option<String>);
}

#[derive(Debug, Clone, Default)]
pub struct MemoryCommitResult {
    pub added_memories: Vec<String>,
    pub profile_updated: bool,
    pub new_revision: u64,
    pub memory_sync_json: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingToolContext {
    role: AgentRole,
    original_text: String,
    tool_name: String,
    retry_count: u8,
}

pub struct Orchestrator {
    llm_gateway: Arc<dyn LlmGateway>,
    memory_store: Arc<dyn MemoryStore>,
    skill_registry: Arc<SkillRegistry>,
    platform: String,
    pending_tool_calls: Arc<Mutex<HashMap<String, PendingToolContext>>>,
    progress_sink: Arc<Mutex<Option<Arc<dyn OrchestratorProgressSink>>>>,
    pub preferred_lang: String,
}

include!("basics.rs");
include!("tool_calls.rs");
include!("memory_and_workers.rs");
include!("pipeline.rs");

#[cfg(test)]
mod tests;

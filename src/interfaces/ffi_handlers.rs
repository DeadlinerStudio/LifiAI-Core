use std::sync::Arc;
use tokio::sync::Mutex;
use std::path::PathBuf;
use crate::domain::traits::{AgentOrchestrator, MemoryStore, ToolExecutor};
use crate::infrastructure::llm_client::DeepSeekClient;
use crate::infrastructure::memory_store::LocalMemoryStore;
use crate::application::orchestrator::Orchestrator;
use crate::interfaces::events::{CoreEvent, FFITask, FFIHabit};
use crate::domain::models::MixedResult;

// UniFFI 会根据 UDL 自动生成这部分 Trait 绑定
pub trait CoreCallback: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

pub struct DeadlinerCore {
    orchestrator: Arc<Orchestrator>,
    memory_store: Arc<LocalMemoryStore>,
    callback: Mutex<Option<Box<dyn CoreCallback>>>,
}

impl DeadlinerCore {
    pub fn new(api_key: String, base_url: String, model_id: String, storage_path: String) -> Self {
        let storage_dir = PathBuf::from(storage_path);
        if !storage_dir.exists() {
            let _ = std::fs::create_dir_all(&storage_dir);
        }

        let memory_store = Arc::new(LocalMemoryStore::new(storage_dir));
        let llm_gateway = Box::new(DeepSeekClient::new(api_key, base_url, model_id));
        let orchestrator = Arc::new(Orchestrator::new(
            llm_gateway, 
            memory_store.clone() as Arc<dyn MemoryStore>, 
            "zh-CN".to_string()
        ));

        Self {
            orchestrator,
            memory_store,
            callback: Mutex::new(None),
        }
    }

    pub fn set_callback(&self, callback: Box<dyn CoreCallback>) {
        // 使用同步阻塞或在一个单独的运行时处理，因为 UniFFI 的 constructor 后的 setter 通常是同步的
        // 这里我们简单地用同步锁
        let mut cb = self.callback.blocking_lock();
        *cb = Some(callback);
    }

    pub async fn process_input(&self, text: String) {
        self.send_event(CoreEvent::OnThinking { agent_name: "Deadliner Supervisor".to_string() }).await;

        match self.orchestrator.process_message(text).await {
            Ok(result) => self.handle_mixed_result(result).await,
            Err(e) => self.send_event(CoreEvent::OnError { message: e.to_string() }).await,
        }
    }

    pub async fn submit_tool_result(&self, id: String, result_json: String) {
        self.send_event(CoreEvent::OnThinking { agent_name: "Tool Executor".to_string() }).await;

        match self.orchestrator.submit_tool_result(id, result_json).await {
            Ok(result) => self.handle_mixed_result(result).await,
            Err(e) => self.send_event(CoreEvent::OnError { message: e.to_string() }).await,
        }
    }

    pub fn get_latest_tasks(&self) -> Vec<FFITask> {
        // 实际上应该从本地数据库查询，这里先返回空或者从内存状态中获取
        vec![]
    }

    pub fn get_user_profile(&self) -> String {
        // 同步获取（实际可能需要阻塞等待异步结果，或者改为异步）
        futures::executor::block_on(self.memory_store.get_user_profile()).unwrap_or_default()
    }

    // 私有辅助方法
    async fn handle_mixed_result(&self, result: MixedResult) {
        // 1. 检查是否有工具调用
        if let Some(tool_calls) = result.tool_calls {
            if let Some(tool) = tool_calls.first() {
                self.send_event(CoreEvent::OnToolRequest { 
                    id: uuid::Uuid::new_v4().to_string(), 
                    tool_name: tool.tool.clone(),
                    args_json: tool.args.to_string(),
                }).await;
                return;
            }
        }

        // 2. 转换并下发最终结果
        let tasks = result.tasks.map(|ts| ts.into_iter().map(|t| FFITask {
            name: t.name,
            due_time: t.due_time,
            note: t.note,
        }).collect());

        let habits = result.habits.map(|hs| hs.into_iter().map(|h| FFIHabit {
            name: h.name,
            period: h.period,
            times_per_period: h.times_per_period,
            goal_type: h.goal_type,
            total_target: h.total_target,
        }).collect());

        self.send_event(CoreEvent::OnFinish { 
            primary_intent: result.primary_intent.unwrap_or_default(),
            tasks,
            habits,
            chat_response: result.chat_response,
            session_summary: result.session_summary,
        }).await;
    }

    async fn send_event(&self, event: CoreEvent) {
        let cb = self.callback.lock().await;
        if let Some(callback) = cb.as_ref() {
            callback.on_event(event);
        }
    }
}

// 对应 UDL 的顶层函数（用于 extract_tasks）
pub fn extract_tasks(text: String, _preferred_lang: String) -> Result<Vec<FFITask>, CoreError> {
    // 这是一个 legacy API 模拟，实际应调用独立的 TaskAgent
    Err(CoreError::ParsingFailed)
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
}

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dotenvy::from_path_override;
use serde_json::json;
use tokio::sync::Mutex;

use core_logic::application::orchestrator::Orchestrator;
use core_logic::domain::models::{
    AIError, AgentRole, ChatChoice, ChatMessage, ChatRequest, ChatResponse, MemoryFragment,
    MixedResult, ToolMetadata,
};
use core_logic::domain::traits::{AgentOrchestrator, LlmGateway, MemoryStore, ToolExecutor};
use core_logic::infrastructure::llm_client::DeepSeekClient;
use core_logic::infrastructure::memory_store::LocalMemoryStore;
use core_logic::infrastructure::skill_registry::SkillRegistry;

struct MockLlmGateway {
    model: String,
    responses: Mutex<VecDeque<String>>,
}

impl MockLlmGateway {
    fn new(responses: Vec<String>) -> Self {
        Self {
            model: "mock-model".to_string(),
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }
}

#[async_trait]
impl LlmGateway for MockLlmGateway {
    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, AIError> {
        let mut responses = self.responses.lock().await;
        let content = responses
            .pop_front()
            .ok_or_else(|| AIError::NetworkError("mock responses exhausted".to_string()))?;

        Ok(ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content,
                },
            }],
        })
    }

    fn get_model_name(&self) -> String {
        self.model.clone()
    }
}

struct MockMemoryStore;

#[async_trait]
impl MemoryStore for MockMemoryStore {
    async fn save_memory(&self, _content: String, _category: String) -> Result<(), AIError> {
        Ok(())
    }

    async fn get_all_fragments(&self) -> Result<Vec<MemoryFragment>, AIError> {
        Ok(vec![])
    }

    async fn save_user_profile(&self, _profile: String) -> Result<(), AIError> {
        Ok(())
    }

    async fn get_user_profile(&self) -> Result<String, AIError> {
        Ok(String::new())
    }

    async fn get_long_term_context(&self) -> Result<String, AIError> {
        Ok(String::new())
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn build_demo_registry() -> Arc<SkillRegistry> {
    Arc::new(SkillRegistry::from_skills(vec![ToolMetadata {
        name: "read_tasks".to_string(),
        description: "读取任务列表，用于判断是否重复。".to_string(),
        domain: AgentRole::TaskAgent,
        supported_platforms: vec![
            "ios".to_string(),
            "android".to_string(),
            "harmony".to_string(),
        ],
        requires_ui_confirmation: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "timeRangeDays": { "type": "integer" },
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        }),
    }]))
}

fn print_result(label: &str, result: &MixedResult) {
    println!("\n{}", label);
    println!("  - 主要意图: {:?}", result.primary_intent);
    println!("  - 路由到的专家: {:?}", result.routed_agents);

    if let Some(tool_calls) = &result.tool_calls {
        for tool_call in tool_calls {
            println!(
                "  - [ToolCall] agent={:?}, tool={}, args={}",
                tool_call.agent, tool_call.tool, tool_call.args
            );
        }
    }

    if let Some(tasks) = &result.tasks {
        for task in tasks {
            println!("  - [Task] {} (截止: {:?})", task.name, task.due_time);
        }
    }

    if let Some(habits) = &result.habits {
        for habit in habits {
            println!(
                "  - [Habit] {} (周期: {}, 次数: {})",
                habit.name, habit.period, habit.times_per_period
            );
        }
    }

    if let Some(chat_response) = &result.chat_response {
        println!("  - [Chat] {}", chat_response);
    }
}

async fn run_mock_demo(platform: String) -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 运行本地 Mock 演示模式...");

    let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
        json!({
            "primaryIntent": "ExtractTasks",
            "routedAgents": ["TaskAgent"]
        })
        .to_string(),
        json!({
            "toolCalls": [
                {
                    "tool": "read_tasks",
                    "args": {
                        "timeRangeDays": 7,
                        "keywords": ["Rust", "冒烟测试"]
                    },
                    "reason": "需要先检查近期任务，避免重复"
                }
            ]
        })
        .to_string(),
        json!({
            "tasks": [
                {
                    "name": "写 Rust 冒烟测试",
                    "dueTime": "2026-03-19 10:00",
                    "note": "确认无重复后创建"
                }
            ],
            "chatResponse": "已经帮你整理好了。"
        })
        .to_string(),
    ]));

    let memory_store: Arc<dyn MemoryStore> = Arc::new(MockMemoryStore);
    let orchestrator = Orchestrator::new(
        llm_gateway,
        memory_store,
        build_demo_registry(),
        platform,
        "zh-CN".to_string(),
    );

    let user_input = "帮我记一下明天上午十点写 Rust 冒烟测试";
    println!("\n👤 用户输入: {}", user_input);

    let initial_result = orchestrator.process_message(user_input.to_string()).await?;
    print_result("🤖 首轮结果:", &initial_result);

    if let Some(tool_call) = initial_result
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .cloned()
    {
        let tool_call_id = "demo-tool-call-1".to_string();
        println!("\n🧰 模拟本地工具执行: {}", tool_call.tool);
        println!("  - toolCallId: {}", tool_call_id);
        println!("  - args: {}", tool_call.args);

        orchestrator
            .register_pending_tool_call(
                tool_call_id.clone(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input.to_string(),
            )
            .await;

        let mock_observation = json!({
            "tasks": []
        })
        .to_string();
        println!("  - observation: {}", mock_observation);

        let final_result = orchestrator
            .submit_tool_result(tool_call_id, mock_observation)
            .await?;
        print_result("✅ 回灌后二次推理结果:", &final_result);
    }

    Ok(())
}

async fn run_real_mode(platform: String) -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .expect("❌ 请在 .env 中设置 DEEPSEEK_API_KEY");
    let base_url = std::env::var("DEEPSEEK_BASE_URL")
        .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string());
    let model = std::env::var("DEEPSEEK_MODEL")
        .unwrap_or_else(|_| "deepseek-chat".to_string());

    println!("🚀 Deadliner AI Core 正在连接 DeepSeek API (多 Agent 模式)...");

    let root = workspace_root();
    let storage_dir = root.join("temp_storage");
    let skills_dir = root.join("skills");
    if !storage_dir.exists() {
        std::fs::create_dir_all(&storage_dir)?;
    }

    let memory_store: Arc<dyn MemoryStore> = Arc::new(LocalMemoryStore::new(storage_dir));
    let llm_gateway: Arc<dyn LlmGateway> = Arc::new(DeepSeekClient::new(api_key, base_url, model));
    let skill_registry = Arc::new(SkillRegistry::load_from_dir(skills_dir)?);

    let orchestrator = Orchestrator::new(
        llm_gateway,
        memory_store,
        skill_registry,
        platform,
        "zh-CN".to_string(),
    );

    let user_input = "帮我记一下：明天上午十点写 Rust 冒烟测试，顺便帮我养成每天喝八杯水的习惯。";
    println!("\n👤 用户输入: {}", user_input);
    println!("🤖 Supervisor 正在路由，各领域专家正在并行处理...");

    let result = orchestrator.process_message(user_input.to_string()).await?;
    print_result("🤖 AI 最终合并结果:", &result);

    println!("\n✅ 多 Agent 实战测试成功！");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = workspace_root();
    let _ = from_path_override(root.join(".env"));

    let platform = std::env::var("DEADLINER_PLATFORM")
        .unwrap_or_else(|_| "ios".to_string());
    let use_mock = std::env::var("DEADLINER_USE_MOCK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if use_mock {
        run_mock_demo(platform).await
    } else {
        run_real_mode(platform).await
    }
}

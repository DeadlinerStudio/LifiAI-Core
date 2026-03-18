pub mod domain;
pub mod application;
pub mod infrastructure;
pub mod interfaces;

use std::path::PathBuf;
use std::sync::Arc;
use dotenvy::dotenv;
use crate::domain::traits::{AgentOrchestrator, MemoryStore, LlmGateway};
use crate::infrastructure::llm_client::DeepSeekClient;
use crate::infrastructure::memory_store::LocalMemoryStore;
use crate::application::orchestrator::Orchestrator;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 0. 加载 .env
    dotenv().ok();
    
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .expect("❌ 请在 .env 中设置 DEEPSEEK_API_KEY");
    let base_url = std::env::var("DEEPSEEK_BASE_URL")
        .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string());
    let model = std::env::var("DEEPSEEK_MODEL")
        .unwrap_or_else(|_| "deepseek-chat".to_string());

    println!("🚀 Deadliner AI Core 正在连接 DeepSeek API (多 Agent 模式)...");

    // 1. 初始化基础设施
    let storage_dir = PathBuf::from("./temp_storage");
    if !storage_dir.exists() {
        std::fs::create_dir_all(&storage_dir)?;
    }
    
    // Orchestrator 现在需要 Arc
    let memory_store: Arc<dyn MemoryStore> = Arc::new(LocalMemoryStore::new(storage_dir));
    let llm_gateway: Arc<dyn LlmGateway> = Arc::new(DeepSeekClient::new(api_key, base_url, model));

    // 2. 初始化应用层编排器
    let orchestrator = Orchestrator::new(llm_gateway, memory_store, "zh-CN".to_string());

    // 3. 模拟用户输入 (复合指令)
    let user_input = "帮我记一下：明天上午十点写 Rust 冒烟测试，顺便帮我养成每天喝八杯水的习惯。";
    println!("\n👤 用户输入: {}", user_input);

    // 4. 执行核心逻辑
    println!("🤖 Supervisor 正在路由，各领域专家正在并行处理...");
    let result = orchestrator.process_message(user_input.to_string()).await?;

    // 5. 打印执行结果
    println!("\n🤖 AI 最终合并结果:");
    println!("  - 主要意图: {:?}", result.primary_intent);
    println!("  - 路由到的专家: {:?}", result.routed_agents);
    
    if let Some(tasks) = result.tasks {
        for task in tasks {
            println!("  - [Task] 提取任务: {} (截止: {:?})", task.name, task.due_time);
        }
    }
    
    if let Some(habits) = result.habits {
        for habit in habits {
            println!("  - [Habit] 提取习惯: {} (周期: {}, 次数: {})", habit.name, habit.period, habit.times_per_period);
        }
    }

    println!("\n✅ 多 Agent 实战测试成功！");

    Ok(())
}

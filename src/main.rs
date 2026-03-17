pub mod domain;
pub mod application;
pub mod infrastructure;
pub mod interfaces;

use std::path::PathBuf;
use dotenvy::dotenv;
use crate::domain::traits::{AgentOrchestrator, MemoryStore};
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
        .unwrap_or_else(|_| "https://api.deepseek.com".to_string());
    let model = std::env::var("DEEPSEEK_MODEL")
        .unwrap_or_else(|_| "deepseek-chat".to_string());

    println!("🚀 Deadliner AI Core 正在连接 DeepSeek API...");
    println!("  - Base URL: {}", base_url);
    println!("  - Model: {}", model);

    // 1. 初始化基础设施
    let storage_dir = PathBuf::from("./temp_storage");
    if !storage_dir.exists() {
        std::fs::create_dir_all(&storage_dir)?;
    }
    let memory_store = Box::new(LocalMemoryStore::new(storage_dir));
    
    // 初始化真正的 LLM 客户端
    let llm_gateway = Box::new(DeepSeekClient::new(api_key, base_url, model));

    // 2. 初始化应用层编排器
    let orchestrator = Orchestrator::new(llm_gateway, memory_store, "zh-CN".to_string());

    // 3. 模拟用户输入
    let user_input = "帮我记一下：明天上午十点写 Rust 冒烟测试。";
    println!("\n👤 用户输入: {}", user_input);

    // 4. 执行核心逻辑（向真正的 LLM 请求）
    println!("🤖 AI 正在思考并提取（发送网络请求）...");
    let result = orchestrator.process_message(user_input.to_string()).await?;

    // 5. 打印执行结果
    println!("\n🤖 AI 解析结果:");
    println!("  - 意图: {:?}", result.primary_intent);
    
    if let Some(tasks) = result.tasks {
        if tasks.is_empty() {
             println!("  - 提取任务: (无)");
        }
        for task in tasks {
            println!("  - 提取任务: {} (截止: {:?})", task.name, task.due_time);
        }
    }
    
    if let Some(memories) = result.new_memories {
        println!("  - 提取记忆: {:?}", memories);
    }
    
    if let Some(response) = result.chat_response {
        println!("  - AI 聊天回复: {}", response);
    }

    println!("\n💾 检查内存持久化...");
    let check_store = LocalMemoryStore::new(PathBuf::from("./temp_storage"));
    let profile = check_store.get_user_profile().await?;
    let fragments = check_store.get_all_fragments().await?;
    
    println!("  - 磁盘中的画像: {}", profile);
    println!("  - 磁盘中的记忆碎片数: {}", fragments.len());

    println!("\n✅ 实战测试成功！DeepSeek 接入成功。");

    Ok(())
}

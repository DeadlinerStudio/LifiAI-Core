use async_trait::async_trait;
use crate::domain::models::{MixedResult, AIError, ChatRequest, ChatMessage};
use crate::domain::traits::{LlmGateway, AgentOrchestrator, ToolExecutor, MemoryStore};
use crate::application::prompt_builder::PromptBuilder;

pub struct Orchestrator {
    llm_gateway: Box<dyn LlmGateway>,
    memory_store: Box<dyn MemoryStore>,
    pub preferred_lang: String,
}

impl Orchestrator {
    pub fn new(llm_gateway: Box<dyn LlmGateway>, memory_store: Box<dyn MemoryStore>, preferred_lang: String) -> Self {
        Self {
            llm_gateway,
            memory_store,
            preferred_lang,
        }
    }

    /// 清理并提取 JSON 字符串 (对应 Swift 的 extractJsonData + stripCodeFence)
    fn extract_json(raw: &str) -> Result<String, AIError> {
        let mut cleaned = raw.trim();
        
        // 移除 Markdown 代码块标记
        if cleaned.starts_with("```json") {
            cleaned = cleaned.strip_prefix("```json").unwrap_or(cleaned);
        } else if cleaned.starts_with("```") {
            cleaned = cleaned.strip_prefix("```").unwrap_or(cleaned);
        }
        
        if cleaned.ends_with("```") {
            cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned);
        }
        
        let cleaned = cleaned.trim();

        // 寻找第一个 '{' 和最后一个 '}'
        if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
            Ok(cleaned[start..=end].to_string())
        } else {
            Err(AIError::ParsingFailed)
        }
    }
}

#[async_trait]
impl AgentOrchestrator for Orchestrator {
    async fn process_message(&self, text: String) -> Result<MixedResult, AIError> {
        // 1. 获取长期记忆上下文
        let long_term = self.memory_store.get_long_term_context().await?;

        // 2. 构建 Prompt
        let system_prompt = PromptBuilder::build_agent_system_prompt(
            &self.preferred_lang,
            &long_term,
            "", // session_summary
            "", // session_context
            None
        );

        let messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt },
            ChatMessage { role: "user".to_string(), content: text },
        ];

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        // 3. 调用 LLM
        let response = self.llm_gateway.chat(request).await?;
        let content = response.choices.first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        // 4. 解析结果
        let json_str = Self::extract_json(&content)?;
        let result: MixedResult = serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;

        // 5. 持久化新的记忆和画像
        if let Some(memories) = &result.new_memories {
            for mem in memories {
                let _ = self.memory_store.save_memory(mem.clone(), "Auto-Extracted".to_string()).await;
            }
        }

        if let Some(profile) = &result.user_profile {
            if !profile.trim().is_empty() {
                let _ = self.memory_store.save_user_profile(profile.clone()).await;
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl ToolExecutor for Orchestrator {
    async fn submit_tool_result(&self, _tool_call_id: String, result_json: String) -> Result<MixedResult, AIError> {
        let long_term = self.memory_store.get_long_term_context().await?;
        
        let tool_hint = "你正在进行二段对话（工具结果已提供）。\n- 不要再次请求 readTasks。\n- 基于工具结果输出新的 proposal。";
        
        let system_prompt = PromptBuilder::build_agent_system_prompt(
            &self.preferred_lang,
            &long_term, 
            "", 
            "",
            Some(tool_hint)
        );

        let messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt },
            ChatMessage { role: "user".to_string(), content: format!("TOOL_RESULT_JSON:\n{}", result_json) },
        ];

        let request = ChatRequest {
            model: self.llm_gateway.get_model_name(),
            messages,
            temperature: 0.1,
        };

        let response = self.llm_gateway.chat(request).await?;
        let content = response.choices.first()
            .map(|c| c.message.content.clone())
            .ok_or(AIError::EmptyResponse)?;

        let json_str = Self::extract_json(&content)?;
        let result: MixedResult = serde_json::from_str(&json_str).map_err(|_| AIError::ParsingFailed)?;

        // 二段对话也可能产生新记忆
        if let Some(memories) = &result.new_memories {
            for mem in memories {
                let _ = self.memory_store.save_memory(mem.clone(), "Auto-Extracted".to_string()).await;
            }
        }

        Ok(result)
    }
}

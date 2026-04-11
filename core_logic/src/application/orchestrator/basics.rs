impl Orchestrator {
    pub fn new(
        llm_gateway: Arc<dyn LlmGateway>,
        memory_store: Arc<dyn MemoryStore>,
        skill_registry: Arc<SkillRegistry>,
        platform: String,
        preferred_lang: String,
    ) -> Self {
        Self {
            llm_gateway,
            memory_store,
            skill_registry,
            platform,
            pending_tool_calls: Arc::new(Mutex::new(HashMap::new())),
            progress_sink: Arc::new(Mutex::new(None)),
            preferred_lang,
        }
    }

    pub async fn set_progress_sink(&self, sink: Arc<dyn OrchestratorProgressSink>) {
        let mut current = self.progress_sink.lock().await;
        *current = Some(sink);
    }

    async fn emit_thinking(&self, agent_name: &str, phase: &str, message: Option<String>) {
        let sink = {
            let current = self.progress_sink.lock().await;
            current.clone()
        };

        if let Some(sink) = sink {
            sink.on_thinking(agent_name.to_string(), phase.to_string(), message)
                .await;
        }
    }

    /// 清理并提取 JSON 字符串
    fn extract_json(raw: &str) -> Result<String, AIError> {
        let mut cleaned = raw.trim();
        if cleaned.starts_with("```json") {
            cleaned = cleaned.strip_prefix("```json").unwrap_or(cleaned);
        } else if cleaned.starts_with("```") {
            cleaned = cleaned.strip_prefix("```").unwrap_or(cleaned);
        }
        if cleaned.ends_with("```") {
            cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned);
        }
        let cleaned = cleaned.trim();

        if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
            Ok(cleaned[start..=end].to_string())
        } else {
            Err(AIError::ParsingFailed)
        }
    }

    fn fallback_chat_result(text: &str, content: &str) -> PlannerResult {
        let trimmed_text = text.trim();
        let trimmed_content = content.trim();

        let mut result = PlannerResult {
            primary_intent: Some("Chat".to_string()),
            chat_response: Some(trimmed_content.to_string()),
            ..Default::default()
        };

        if Self::looks_like_memory_statement(trimmed_text) {
            result.new_memories = Some(vec![trimmed_text.to_string()]);
        }

        if Self::looks_like_profile_statement(trimmed_text) {
            result.user_profile = Some(trimmed_text.to_string());
        }

        result
    }

    fn looks_like_memory_statement(text: &str) -> bool {
        let patterns = [
            "记住",
            "记一下",
            "以后",
            "我是",
            "我喜欢",
            "我习惯",
            "我通常",
            "请叫我",
        ];
        patterns.iter().any(|pattern| text.contains(pattern))
    }

    fn looks_like_profile_statement(text: &str) -> bool {
        let patterns = [
            "我是",
            "我喜欢",
            "我习惯",
            "我通常",
            "偏好",
            "请叫我",
            "默认",
        ];
        patterns.iter().any(|pattern| text.contains(pattern))
    }

    fn looks_like_task_query(text: &str) -> bool {
        let normalized = text.to_lowercase();
        let exclusion_patterns = ["重复", "冲突", "是否已有", "有没有重复", "避免重复", "去重"];
        let direct_patterns = [
            "有哪些任务",
            "有什么任务",
            "任务列表",
            "列出任务",
            "看看任务",
            "查看任务",
            "总结任务",
            "哪些待办",
            "有什么待办",
            "最近有什么任务",
            "这周有什么任务",
            "本周有什么任务",
            "read my tasks",
            "show my tasks",
            "list my tasks",
            "view my tasks",
            "what are my tasks",
            "what's on my task list",
            "task list",
            "my tasks",
        ];
        let query_verbs = [
            "列出",
            "查看",
            "看看",
            "总结",
            "查询",
            "盘点",
            "read",
            "show",
            "list",
            "view",
            "summarize",
        ];
        let task_nouns = ["任务", "待办", "deadline", "ddl", "todo", "task", "tasks"];

        if exclusion_patterns
            .iter()
            .any(|pattern| normalized.contains(pattern))
        {
            return false;
        }

        direct_patterns
            .iter()
            .any(|pattern| normalized.contains(pattern))
            || (query_verbs.iter().any(|verb| normalized.contains(verb))
                && task_nouns.iter().any(|noun| normalized.contains(noun)))
    }

    fn looks_like_habit_query(text: &str) -> bool {
        let normalized = text.to_lowercase();
        let patterns = [
            "有哪些习惯",
            "我的习惯",
            "习惯列表",
            "查看习惯",
            "看看习惯",
            "总结习惯",
        ];
        patterns.iter().any(|pattern| normalized.contains(pattern))
    }

    fn is_query_request(role: &AgentRole, text: &str) -> bool {
        match role {
            AgentRole::TaskAgent => Self::looks_like_task_query(text),
            AgentRole::HabitAgent => Self::looks_like_habit_query(text),
            _ => false,
        }
    }

    fn summarize_tasks_for_query(tasks: &[crate::domain::models::AITask]) -> String {
        if tasks.is_empty() {
            return "当前没有查到相关任务。".to_string();
        }

        let summary = tasks
            .iter()
            .take(5)
            .map(|task| match task.due_time.as_deref() {
                Some(due) if !due.is_empty() => format!("{}（{}）", task.name, due),
                _ => task.name.clone(),
            })
            .collect::<Vec<_>>()
            .join("；");

        let suffix = if tasks.len() > 5 {
            "；其余任务已省略。"
        } else {
            "。"
        };
        format!(
            "我查到你当前有 {} 个相关任务：{}{}",
            tasks.len(),
            summary,
            suffix
        )
    }

    fn summarize_habits_for_query(habits: &[crate::domain::models::AIHabit]) -> String {
        if habits.is_empty() {
            return "当前没有查到相关习惯。".to_string();
        }

        let summary = habits
            .iter()
            .take(5)
            .map(|habit| format!("{}（{}）", habit.name, habit.period))
            .collect::<Vec<_>>()
            .join("；");

        let suffix = if habits.len() > 5 {
            "；其余习惯已省略。"
        } else {
            "。"
        };
        format!(
            "我查到你当前有 {} 个相关习惯：{}{}",
            habits.len(),
            summary,
            suffix
        )
    }

    fn normalize_query_result(role: &AgentRole, original_text: &str, result: &mut MixedResult) {
        if !Self::is_query_request(role, original_text) {
            return;
        }

        match role {
            AgentRole::TaskAgent => {
                let tasks = result.tasks.take().unwrap_or_default();
                if result
                    .chat_response
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    result.chat_response = Some(Self::summarize_tasks_for_query(&tasks));
                }
                result.retrieved_tasks = Some(tasks);
                result.tasks = Some(vec![]);
            }
            AgentRole::HabitAgent => {
                let habits = result.habits.take().unwrap_or_default();
                if result
                    .chat_response
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    result.chat_response = Some(Self::summarize_habits_for_query(&habits));
                }
                result.retrieved_habits = Some(habits);
                result.habits = Some(vec![]);
            }
            _ => {}
        }
    }

}

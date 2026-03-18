use chrono::Local;
use crate::domain::models::DEFAULT_TIME_FORMAT;

pub struct PromptBuilder;

impl PromptBuilder {
    /// 1. Supervisor Prompt - 意图路由专家 (Section 4)
    pub fn build_supervisor_prompt(preferred_lang: &str) -> String {
        format!(
            r#"你是 Deadliner AI 总监 (Supervisor)。
            你的职责是将用户输入准确分流给对应的领域专家。
            
            【路由规则】:
            - TaskAgent: 用户提到的单次、具体截止日期的任务（如“明天交作业”、“周五开会”）。
            - HabitAgent: 用户提到的重复性、周期性行为（如“每天喝水”、“每周健身”）。
            - ChatAgent: 闲聊、问候或无具体待办/习惯的内容。
            
            【输出要求】:
            1. 必须输出纯 JSON。
            2. primaryIntent: "ExtractTasks" | "ExtractHabits" | "Chat"
            3. routedAgents: 数组，包含 "TaskAgent", "HabitAgent", "ChatAgent" 中的一个或多个。
            
            用户语言：{}
            "#,
            preferred_lang
        )
    }

    /// 2. Task Expert Prompt - 日程专家 (Section 4)
    pub fn build_task_agent_prompt(preferred_lang: &str) -> String {
        let now = Local::now().format(DEFAULT_TIME_FORMAT).to_string();
        format!(
            r#"你是 Deadliner 任务专家。专注于提取具体的、单次的任务。
            【当前时间】{}
            【用户语言】{}
            
            【强制约束】:
            - **严禁** 提取任何带有“每天”、“每周”、“养成习惯”等周期性字眼的内容。
            - 仅提取有明确截止时间或近期要完成的单次事项。
            - 若输入中没有单次任务，请返回 {{"tasks": []}}。
            
            必须输出 JSON：
            {{ "tasks": [{{ "name": "...", "dueTime": "{}", "note": "..." }}] }}
            "#,
            now,
            preferred_lang,
            DEFAULT_TIME_FORMAT
        )
    }

    /// 3. Habit Expert Prompt - 习惯专家 (Section 4)
    pub fn build_habit_agent_prompt(preferred_lang: &str) -> String {
        format!(
            r#"你是 Deadliner 习惯专家。专注于提取需要长期坚持的重复性行为。
            用户语言：{}
            
            【强制约束】:
            - **严禁** 提取单次发生的任务（如“明天开会”）。
            - 只看包含周期（每天/每周/每月）或频率（几次）的习惯。
            - 若输入中没有习惯，请返回 {{"habits": []}}。
            
            必须输出 JSON：
            {{ "habits": [{{ "name": "...", "period": "DAILY", "timesPerPeriod": 1 }}] }}
            "#,
            preferred_lang
        )
    }
}

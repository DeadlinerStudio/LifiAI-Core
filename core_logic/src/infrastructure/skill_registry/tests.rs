use super::*;

#[test]
fn registry_can_be_created_from_skills() {
    let registry = SkillRegistry::from_skills(vec![ToolMetadata {
        name: "test_tool".to_string(),
        description: "test".to_string(),
        domain: AgentRole::TaskAgent,
        capabilities: vec![ToolCapability::ReadTaskContext],
        supported_platforms: vec!["ios".to_string()],
        requires_ui_confirmation: false,
        parameters: json!({}),
    }]);

    let resolved = registry.resolve_execution_plan(
        &AgentRole::TaskAgent,
        "ios",
        &ToolCapability::ReadTaskContext,
        json!({}),
        None,
    );

    let plan = resolved.expect("test_tool should be available");
    assert_eq!(plan.tool_name, "test_tool");
}

#[test]
fn registry_exposes_habit_and_task_capabilities() {
    let registry = SkillRegistry::from_skills(vec![
        ToolMetadata {
            name: "read_tasks".to_string(),
            description: "read tasks".to_string(),
            domain: AgentRole::TaskAgent,
            capabilities: vec![ToolCapability::ReadTaskContext],
            supported_platforms: vec!["ios".to_string()],
            requires_ui_confirmation: false,
            parameters: json!({}),
        },
        ToolMetadata {
            name: "read_habits".to_string(),
            description: "read habits".to_string(),
            domain: AgentRole::HabitAgent,
            capabilities: vec![ToolCapability::ReadHabitContext],
            supported_platforms: vec!["ios".to_string()],
            requires_ui_confirmation: false,
            parameters: json!({}),
        },
    ]);

    let task_candidates = registry.candidates_for_capability(
        &AgentRole::TaskAgent,
        "ios",
        &ToolCapability::ReadTaskContext,
    );
    let habit_candidates = registry.candidates_for_capability(
        &AgentRole::HabitAgent,
        "ios",
        &ToolCapability::ReadHabitContext,
    );

    assert_eq!(task_candidates.len(), 1);
    assert_eq!(task_candidates[0].tool.name, "read_tasks");
    assert_eq!(habit_candidates.len(), 1);
    assert_eq!(habit_candidates[0].tool.name, "read_habits");
}

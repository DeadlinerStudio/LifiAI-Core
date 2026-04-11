use super::*;

#[test]
fn falls_back_to_builtin_registry_when_dir_is_missing() {
    let registry = SkillRegistry::load_from_dir_or_builtin("/tmp/deadliner-missing-skills-dir")
        .expect("registry should load");

    let resolved = registry.resolve_execution_plan(
        &AgentRole::TaskAgent,
        "ios",
        &ToolCapability::ReadTaskContext,
        json!({}),
        None,
    );

    let plan = resolved.expect("builtin read_tasks should be available");
    assert_eq!(plan.tool_name, "read_tasks");
    assert_eq!(plan.execution_mode, CapabilityExecutionMode::Auto);
}

#[test]
fn builtin_registry_exposes_habit_and_task_capabilities() {
    let registry = SkillRegistry::builtin();

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

use std::collections::HashMap;

use crate::interfaces::events::CoreEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Started,
    WaitingTool,
    OrchestrationCompleted,
    MemoryCommitted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootStage {
    ProcessInput,
    SubmitToolResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequestLifecycle {
    root_stage: RootStage,
    state: RequestState,
}

/// Validate lifecycle events emitted by CoreEvent::OnLifecycle.
/// Returns Ok when every request_id follows the lifecycle contract.
pub fn assert_lifecycle_sequence(events: &[CoreEvent]) -> Result<(), String> {
    let mut states: HashMap<&str, RequestLifecycle> = HashMap::new();

    for event in events {
        let CoreEvent::OnLifecycle {
            request_id,
            stage,
            status,
            ..
        } = event
        else {
            continue;
        };

        let key = request_id.as_str();
        match states.get(key).copied() {
            None => {
                let root_stage = match (stage.as_str(), status.as_str()) {
                    ("process_input", "started") => RootStage::ProcessInput,
                    ("submit_tool_result", "started") => RootStage::SubmitToolResult,
                    _ => {
                        return Err(format!(
                            "request {} first lifecycle event must be *_started, got {}.{}",
                            request_id, stage, status
                        ));
                    }
                };
                states.insert(
                    key,
                    RequestLifecycle {
                        root_stage,
                        state: RequestState::Started,
                    },
                );
            }
            Some(current) => {
                let next = transition(current, stage, status).map_err(|msg| {
                    format!(
                        "request {} lifecycle invalid transition: {}",
                        request_id, msg
                    )
                })?;
                states.insert(
                    key,
                    RequestLifecycle {
                        root_stage: current.root_stage,
                        state: next,
                    },
                );
            }
        }
    }

    Ok(())
}

fn transition(
    current: RequestLifecycle,
    stage: &str,
    status: &str,
) -> Result<RequestState, String> {
    use RequestState::*;

    match (current.state, stage, status) {
        (Started, "tool_request", "waiting") => Ok(WaitingTool),
        (Started, "orchestration", "completed") => Ok(OrchestrationCompleted),
        (Started, s, "failed") if is_failed_stage_for_root(current.root_stage, s) => Ok(Failed),

        (WaitingTool, "tool_request", "waiting") => Ok(WaitingTool),
        (WaitingTool, s, "failed") if is_failed_stage_for_root(current.root_stage, s) => Ok(Failed),

        (OrchestrationCompleted, "memory_commit", "completed") => Ok(MemoryCommitted),
        (OrchestrationCompleted, "memory_commit", "failed") => Ok(Failed),

        (MemoryCommitted, _, _) => {
            Err("no events allowed after memory_commit.completed".to_string())
        }
        (Failed, _, _) => Err("no events allowed after *.failed".to_string()),

        _ => Err(format!(
            "state={:?} does not allow {}.{}",
            current.state, stage, status
        )),
    }
}

fn is_failed_stage_for_root(root: RootStage, stage: &str) -> bool {
    match root {
        RootStage::ProcessInput => matches!(stage, "process_input" | "memory_commit"),
        RootStage::SubmitToolResult => matches!(stage, "submit_tool_result" | "memory_commit"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn life(request_id: &str, stage: &str, status: &str) -> CoreEvent {
        CoreEvent::OnLifecycle {
            request_id: request_id.to_string(),
            stage: stage.to_string(),
            status: status.to_string(),
            message: None,
        }
    }

    #[test]
    fn accepts_process_input_complete_sequence() {
        let events = vec![
            life("req-1", "process_input", "started"),
            life("req-1", "orchestration", "completed"),
            life("req-1", "memory_commit", "completed"),
        ];
        assert!(assert_lifecycle_sequence(&events).is_ok());
    }

    #[test]
    fn accepts_waiting_tool_branch() {
        let events = vec![
            life("req-2", "process_input", "started"),
            life("req-2", "tool_request", "waiting"),
        ];
        assert!(assert_lifecycle_sequence(&events).is_ok());
    }

    #[test]
    fn accepts_submit_tool_result_failure() {
        let events = vec![
            life("req-3", "submit_tool_result", "started"),
            life("req-3", "submit_tool_result", "failed"),
        ];
        assert!(assert_lifecycle_sequence(&events).is_ok());
    }

    #[test]
    fn rejects_memory_before_orchestration() {
        let events = vec![
            life("req-4", "process_input", "started"),
            life("req-4", "memory_commit", "completed"),
        ];
        assert!(assert_lifecycle_sequence(&events).is_err());
    }

    #[test]
    fn rejects_events_after_terminal_state() {
        let events = vec![
            life("req-5", "submit_tool_result", "started"),
            life("req-5", "submit_tool_result", "failed"),
            life("req-5", "orchestration", "completed"),
        ];
        assert!(assert_lifecycle_sequence(&events).is_err());
    }

    #[test]
    fn rejects_invalid_first_event() {
        let events = vec![life("req-6", "orchestration", "completed")];
        assert!(assert_lifecycle_sequence(&events).is_err());
    }
}

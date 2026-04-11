use core_logic::interfaces::events::CoreEvent;
use core_logic::interfaces::lifecycle_contract::assert_lifecycle_sequence;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleEventSnapshot {
    request_id: String,
    stage: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct LifecycleCase {
    name: String,
    events: Vec<LifecycleEventSnapshot>,
}

#[derive(Debug, Deserialize)]
struct LifecycleFixture {
    version: u32,
    cases: Vec<LifecycleCase>,
}

#[test]
fn lifecycle_snapshot_fixture_matches_contract() {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lifecycle_sequences.json");
    let fixture_content =
        std::fs::read_to_string(fixture_path).expect("should read lifecycle snapshot fixture");
    let fixture: LifecycleFixture =
        serde_json::from_str(&fixture_content).expect("fixture json should be valid");

    assert_eq!(fixture.version, 1, "unexpected lifecycle fixture version");
    assert!(
        !fixture.cases.is_empty(),
        "lifecycle fixture cases should not be empty"
    );

    for case in fixture.cases {
        let events = case
            .events
            .into_iter()
            .map(|event| CoreEvent::OnLifecycle {
                request_id: event.request_id,
                stage: event.stage,
                status: event.status,
                message: None,
            })
            .collect::<Vec<_>>();

        assert_lifecycle_sequence(&events).unwrap_or_else(|err| {
            panic!("case {} violated lifecycle contract: {}", case.name, err)
        });
    }
}

#[test]
fn invalid_lifecycle_snapshot_fixture_is_rejected() {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lifecycle_sequences_invalid.json");
    let fixture_content = std::fs::read_to_string(fixture_path)
        .expect("should read invalid lifecycle snapshot fixture");
    let fixture: LifecycleFixture =
        serde_json::from_str(&fixture_content).expect("invalid fixture json should still be valid");

    assert_eq!(
        fixture.version, 1,
        "unexpected invalid lifecycle fixture version"
    );
    assert!(
        !fixture.cases.is_empty(),
        "invalid lifecycle fixture cases should not be empty"
    );

    for case in fixture.cases {
        let events = case
            .events
            .into_iter()
            .map(|event| CoreEvent::OnLifecycle {
                request_id: event.request_id,
                stage: event.stage,
                status: event.status,
                message: None,
            })
            .collect::<Vec<_>>();

        assert!(
            assert_lifecycle_sequence(&events).is_err(),
            "case {} should be rejected by lifecycle contract",
            case.name
        );
    }
}

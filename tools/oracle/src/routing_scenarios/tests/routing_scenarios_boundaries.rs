use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::{MAX_CONCURRENT, MAX_STEPS, Scenario, ScenarioStep, parse_scenario};

const HTTP_STEP: &str =
    r#"{"http":{"request":{"method":"POST","path":"/a","headers":{},"body":{}}}}"#;
const JOIN: &str = r#"{"join":{}}"#;

fn concurrent(requests: usize) -> String {
    let request = r#"{"request":{"method":"POST","path":"/a","headers":{},"body":{}}}"#;
    format!(
        r#"{{"concurrent":{{"requests":[{}]}}}}"#,
        vec![request; requests].join(",")
    )
}

fn scenario(steps: &str) -> Vec<u8> {
    format!(r#"{{"steps":[{steps}]}}"#).into_bytes()
}

fn seed(kind: &str) -> Vec<u8> {
    let name = format!("fuzz/corpus/scenario/routing-scenarios-{kind}.json");
    fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name)).unwrap()
}

enum Expected {
    Error(&'static str),
    Parsed(fn(&Scenario)),
}

fn assert_256_http_steps(scenario: &Scenario) {
    assert_eq!(scenario.steps.len(), MAX_STEPS);
    assert!(
        scenario
            .steps
            .iter()
            .all(|step| matches!(step, ScenarioStep::Http(_)))
    );
}

fn assert_32_concurrent_requests(scenario: &Scenario) {
    assert_eq!(scenario.steps.len(), 2);
    match &scenario.steps[0] {
        ScenarioStep::Concurrent(step) => assert_eq!(step.requests.len(), MAX_CONCURRENT),
        other @ (ScenarioStep::Http(_) | ScenarioStep::Join(_)) => {
            panic!("expected a concurrent step, got {other:?}")
        }
    }
}

fn assert_seed_shape(scenario: &Scenario) {
    assert_eq!(scenario.steps.len(), 3);
    assert!(matches!(scenario.steps[0], ScenarioStep::Http(_)));
    match &scenario.steps[1] {
        ScenarioStep::Concurrent(step) => assert_eq!(step.requests.len(), 2),
        other @ (ScenarioStep::Http(_) | ScenarioStep::Join(_)) => {
            panic!("expected a concurrent step, got {other:?}")
        }
    }
    match &scenario.steps[2] {
        ScenarioStep::Join(join) => assert_eq!(join.deadline_ms, 1000),
        other @ (ScenarioStep::Http(_) | ScenarioStep::Concurrent(_)) => {
            panic!("expected a join step, got {other:?}")
        }
    }
}

#[rstest]
#[case::empty_steps(
    scenario(""),
    Expected::Error("invalid scenario: scenario.steps cannot be empty")
)]
#[case::steps_at_limit(
    scenario(&vec![HTTP_STEP; MAX_STEPS].join(",")),
    Expected::Parsed(assert_256_http_steps)
)]
#[case::steps_past_limit(
    scenario(&vec![HTTP_STEP; MAX_STEPS + 1].join(",")),
    Expected::Error("invalid scenario: scenario.steps cannot exceed 256")
)]
#[case::concurrent_at_limit(
    scenario(&format!("{},{}", concurrent(MAX_CONCURRENT), JOIN)),
    Expected::Parsed(assert_32_concurrent_requests)
)]
#[case::concurrent_past_limit(
    scenario(&concurrent(MAX_CONCURRENT + 1)),
    Expected::Error("invalid scenario: concurrent.requests cannot exceed 32")
)]
#[case::concurrent_needs_two(
    scenario(&concurrent(1)),
    Expected::Error("invalid scenario: concurrent.requests needs at least two requests")
)]
#[case::concurrent_before_join(
    scenario(&format!("{},{}", concurrent(2), concurrent(2))),
    Expected::Error("invalid scenario: a concurrent step cannot start before the previous one joins")
)]
#[case::join_without_concurrent(
    scenario(JOIN),
    Expected::Error("invalid scenario: join has no concurrent step to join")
)]
#[case::unjoined_concurrent(
    scenario(&concurrent(2)),
    Expected::Error("invalid scenario: scenario ends with an unjoined concurrent step")
)]
#[case::join_deadline_zero(
    scenario(&format!("{},{{\"join\":{{\"deadline_ms\":0}}}}", concurrent(2))),
    Expected::Error("invalid scenario: join.deadline_ms 0 is outside 1 to 30000")
)]
#[case::join_deadline_past_limit(
    scenario(&format!(
        "{},{{\"join\":{{\"deadline_ms\":30001}}}}",
        concurrent(2)
    )),
    Expected::Error("invalid scenario: join.deadline_ms 30001 is outside 1 to 30000")
)]
#[case::empty_request_path(
    scenario(r#"{"http":{"request":{"method":"POST","path":"","headers":{},"body":{}}}}"#),
    Expected::Error("invalid scenario: scenario request.path cannot be empty")
)]
#[case::unknown_field(
    scenario(
        r#"{"http":{"request":{"method":"POST","path":"/a","headers":{},"body":{}},"bogus":true}}"#
    ),
    Expected::Error(
        "cannot parse scenario: unknown field `bogus`, expected `request` or `compare` at line 1 column 89"
    )
)]
#[case::oversized(
    vec![b' '; 16 * 1024 * 1024 + 1],
    Expected::Error("scenario is 16777217 bytes, past the 16 MiB limit")
)]
#[case::valid_seed(seed("valid"), Expected::Parsed(assert_seed_shape))]
#[case::invalid_seed(
    seed("invalid"),
    Expected::Error("invalid scenario: join has no concurrent step to join")
)]
fn parser_enforces_every_bound(#[case] bytes: Vec<u8>, #[case] expected: Expected) {
    match (parse_scenario(&bytes), expected) {
        (Ok(scenario), Expected::Parsed(check)) => check(&scenario),
        (Err(failure), Expected::Error(message)) => assert_eq!(failure.to_string(), message),
        (Ok(_), Expected::Error(message)) => {
            panic!("expected parsing to fail with \"{message}\", but it succeeded")
        }
        (Err(failure), Expected::Parsed(_)) => {
            panic!("expected parsing to succeed, but it failed with \"{failure}\"")
        }
    }
}

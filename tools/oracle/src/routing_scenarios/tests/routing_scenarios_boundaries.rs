use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::{MAX_CONCURRENT, MAX_STEPS, parse_scenario};

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

#[rstest]
#[case::empty_steps(scenario(""), "invalid scenario: scenario.steps cannot be empty")]
#[case::steps_at_limit(scenario(&vec![HTTP_STEP; MAX_STEPS].join(",")), "")]
#[case::steps_past_limit(
    scenario(&vec![HTTP_STEP; MAX_STEPS + 1].join(",")),
    "invalid scenario: scenario.steps cannot exceed 256"
)]
#[case::concurrent_at_limit(
    scenario(&format!("{},{}", concurrent(MAX_CONCURRENT), JOIN)),
    ""
)]
#[case::concurrent_past_limit(
    scenario(&concurrent(MAX_CONCURRENT + 1)),
    "invalid scenario: concurrent.requests cannot exceed 32"
)]
#[case::concurrent_needs_two(
    scenario(&concurrent(1)),
    "invalid scenario: concurrent.requests needs at least two requests"
)]
#[case::concurrent_before_join(
    scenario(&format!("{},{}", concurrent(2), concurrent(2))),
    "invalid scenario: a concurrent step cannot start before the previous one joins"
)]
#[case::join_without_concurrent(
    scenario(JOIN),
    "invalid scenario: join has no concurrent step to join"
)]
#[case::unjoined_concurrent(
    scenario(&concurrent(2)),
    "invalid scenario: scenario ends with an unjoined concurrent step"
)]
#[case::join_deadline_zero(
    scenario(&format!("{},{{\"join\":{{\"deadline_ms\":0}}}}", concurrent(2))),
    "invalid scenario: join.deadline_ms 0 is outside 1 to 30000"
)]
#[case::join_deadline_past_limit(
    scenario(&format!(
        "{},{{\"join\":{{\"deadline_ms\":30001}}}}",
        concurrent(2)
    )),
    "invalid scenario: join.deadline_ms 30001 is outside 1 to 30000"
)]
#[case::empty_request_path(
    scenario(r#"{"http":{"request":{"method":"POST","path":"","headers":{},"body":{}}}}"#),
    "invalid scenario: scenario request.path cannot be empty"
)]
#[case::unknown_field(
    scenario(
        r#"{"http":{"request":{"method":"POST","path":"/a","headers":{},"body":{}},"bogus":true}}"#
    ),
    "cannot parse scenario: unknown field `bogus`"
)]
#[case::oversized(vec![b' '; 16 * 1024 * 1024 + 1], "scenario is 16777217 bytes, past the 16 MiB limit")]
#[case::valid_seed(seed("valid"), "")]
#[case::invalid_seed(
    seed("invalid"),
    "invalid scenario: join has no concurrent step to join"
)]
fn parser_enforces_every_bound(#[case] bytes: Vec<u8>, #[case] error: &str) {
    let parsed = parse_scenario(&bytes).map(|_scenario| String::new());
    assert!(
        parsed
            .unwrap_or_else(|failure| failure.to_string())
            .starts_with(error)
    );
}

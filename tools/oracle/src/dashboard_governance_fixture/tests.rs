#![cfg(test)]

use std::path::Path;

use betterlite_postgres_fixture::dashboard_governance::DashboardGovernanceSeed;

use crate::case::{Case, decode_case};

use self::seed::case_with;

mod digest;
mod digest_discovery;
#[path = "case/tests/seed.rs"]
mod seed;

const VALID_GOVERNANCE: &str = "    dashboard_governance:\n      metrics:\n        - {guardrail_id: gr-pii, date: '2026-01-04', requests_evaluated: 10, passed_count: 7, blocked_count: 2, flagged_count: 1, avg_score: '0.875', avg_latency_ms: '12.5'}\n        - {guardrail_id: gr-config, date: '2026-01-04', requests_evaluated: 3, passed_count: 3}\n      usage_units:\n        - {guardrail_id: gr-pii, date: '2026-01-04', team_id: team-red, key_index: 0, usage_unit: contentPolicyUnits, units: 5, cost: '0.25', untracked_units: 0}\n        - {guardrail_id: gr-pii, date: '2026-01-04', usage_unit: textUnits, units: 3, untracked_units: 3}\n      guardrail_index:\n        - {request_id: req-blocked, guardrail_id: gr-pii, policy_id: policy-eu, start_time: '2026-01-04T10:00:00.000'}\n        - {request_id: req-blocked, guardrail_id: gr-config, start_time: '2026-01-04T10:00:00.000'}\n      tool_index:\n        - {request_id: req-tools, tool_name: mcp__github/search, start_time: '2026-01-04T11:00:00.000'}\n      tools:\n        - {tool_id: tool-github, tool_name: mcp__github/search, origin: user_defined, input_policy: blocked, output_policy: trusted, call_count: 42, key_index: 0, team_id: team-red, key_alias: alice-discovery-key, user_agent: openai-python/1.0, last_used_at: '2026-01-04T11:00:01.000'}\n        - {tool_id: tool-weather, tool_name: get_weather}\n";

fn decoded_governance(case: &Case) -> Option<&DashboardGovernanceSeed> {
    case.database.as_ref()?.seed.dashboard_governance.as_ref()
}

fn seed_digest(seed: &DashboardGovernanceSeed) -> Vec<String> {
    let mut lines = digest::measurements(seed);
    lines.extend(digest_discovery::discovery(seed));
    lines
}

fn expected_digest() -> Vec<String> {
    [
        "metric|gr-pii|2026-01-04|10|7|2|1|Some(\"0.875\")|Some(\"12.5\")",
        "metric|gr-config|2026-01-04|3|3|0|0|None|None",
        "unit|gr-pii|2026-01-04|team-red|Some(0)|contentPolicyUnits|5|Some(\"0.25\")|0",
        "unit|gr-pii|2026-01-04||None|textUnits|3|None|3",
        "guard|req-blocked|gr-pii|Some(\"policy-eu\")|2026-01-04T10:00:00.000",
        "guard|req-blocked|gr-config|None|2026-01-04T10:00:00.000",
        "tool_index|req-tools|mcp__github/search|2026-01-04T11:00:00.000",
        "tool|tool-github|mcp__github/search|Some(\"user_defined\")|blocked|trusted|42|Some(0)|Some(\"team-red\")|Some(\"alice-discovery-key\")|Some(\"openai-python/1.0\")|Some(\"2026-01-04T11:00:01.000\")",
        "tool|tool-weather|get_weather|None|untrusted|untrusted|0|None|None|None|None|None",
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

#[test]
fn governance_case_seed_accepts() {
    let yaml = case_with(VALID_GOVERNANCE);
    let case = decode_case(Path::new("<bytes>"), yaml.as_bytes()).expect("the valid case decodes");
    let governance = decoded_governance(&case).expect("the case carries a governance seed");
    assert_eq!(seed_digest(governance), expected_digest());

    let absent = decode_case(Path::new("<bytes>"), case_with("").as_bytes())
        .expect("a case without governance decodes");
    assert!(decoded_governance(&absent).is_none());

    let empty = decode_case(
        Path::new("<bytes>"),
        case_with("    dashboard_governance: {}\n").as_bytes(),
    )
    .expect("an empty governance seed decodes");
    let empty_governance = decoded_governance(&empty).expect("the empty seed decodes to defaults");
    assert!(seed_digest(empty_governance).is_empty());
}

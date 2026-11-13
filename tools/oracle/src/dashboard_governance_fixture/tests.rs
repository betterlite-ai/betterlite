#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_postgres_fixture::dashboard_governance::{
    DashboardGovernanceError, DashboardGovernanceSeed,
};
use betterlite_postgres_fixture::error::FixtureError;

use crate::OracleError;
use crate::case::{Case, decode_case};

mod digest;
mod digest_discovery;

const SEED_PREFIX: &str = "name: governance-fixture\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ndatabase:\n  seed:\n    users: [{user_id: alice}, {user_id: bob}]\n    keys: [{token: sk-alice-token, user_id: alice}, {token: sk-bob-token, user_id: bob}]\n    teams: [{team_id: team-red}]\n    memberships: [{user_id: alice, team_id: team-red}]\n    dashboard:\n      logs:\n        - {request_id: req-blocked, call_type: acompletion, start_time: '2026-01-04T10:00:00.000', end_time: '2026-01-04T10:00:00.500', key_index: 0, status: failure}\n        - {request_id: req-tools, call_type: acompletion, start_time: '2026-01-04T11:00:00.000', end_time: '2026-01-04T11:00:00.400', key_index: 1, status: success}\n";

const CASE_SUFFIX: &str =
    "\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n";

const VALID_GOVERNANCE: &str = "    dashboard_governance:\n      metrics:\n        - {guardrail_id: gr-pii, date: '2026-01-04', requests_evaluated: 10, passed_count: 7, blocked_count: 2, flagged_count: 1, avg_score: '0.875', avg_latency_ms: '12.5'}\n        - {guardrail_id: gr-config, date: '2026-01-04', requests_evaluated: 3, passed_count: 3}\n      usage_units:\n        - {guardrail_id: gr-pii, date: '2026-01-04', team_id: team-red, key_index: 0, usage_unit: contentPolicyUnits, units: 5, cost: '0.25', untracked_units: 0}\n        - {guardrail_id: gr-pii, date: '2026-01-04', usage_unit: textUnits, units: 3, untracked_units: 3}\n      guardrail_index:\n        - {request_id: req-blocked, guardrail_id: gr-pii, policy_id: policy-eu, start_time: '2026-01-04T10:00:00.000'}\n        - {request_id: req-blocked, guardrail_id: gr-config, start_time: '2026-01-04T10:00:00.000'}\n      tool_index:\n        - {request_id: req-tools, tool_name: mcp__github/search, start_time: '2026-01-04T11:00:00.000'}\n      tools:\n        - {tool_id: tool-github, tool_name: mcp__github/search, origin: user_defined, input_policy: blocked, output_policy: trusted, call_count: 42, key_index: 0, team_id: team-red, key_alias: alice-discovery-key, user_agent: openai-python/1.0, last_used_at: '2026-01-04T11:00:01.000'}\n        - {tool_id: tool-weather, tool_name: get_weather}\n";

fn case_with(governance: &str) -> String {
    format!("{SEED_PREFIX}{governance}{CASE_SUFFIX}")
}

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

#[derive(Debug, Clone, Copy)]
enum Rejected {
    MissingDashboard,
    NegativeCounter,
    DuplicateIdentity,
    UnknownLog,
    UnknownKey,
    MissingTeam,
    Parse,
}

fn governance_error(outcome: Result<Case, OracleError>) -> DashboardGovernanceError {
    match outcome {
        Err(OracleError::HttpCaseFixture {
            source: FixtureError::DashboardGovernance(error),
        }) => error,
        other => panic!("expected a governance fixture rejection, got {other:?}"),
    }
}

fn assert_failure(case: &str, error: &DashboardGovernanceError, expected: Rejected) {
    assert!(
        matches!(
            (error, expected),
            (
                DashboardGovernanceError::MissingDashboard,
                Rejected::MissingDashboard
            ) | (
                DashboardGovernanceError::NegativeCounter { .. },
                Rejected::NegativeCounter
            ) | (
                DashboardGovernanceError::DuplicateIdentity { .. },
                Rejected::DuplicateIdentity
            ) | (
                DashboardGovernanceError::UnknownLog { .. },
                Rejected::UnknownLog
            ) | (
                DashboardGovernanceError::UnknownKey { .. },
                Rejected::UnknownKey
            ) | (
                DashboardGovernanceError::MissingTeam { .. },
                Rejected::MissingTeam
            )
        ),
        "{case}: unexpected governance error {error:?}"
    );
}

fn assert_rejection(case: &str, outcome: Result<Case, OracleError>, expected: Rejected) {
    if matches!(expected, Rejected::Parse) {
        assert!(
            matches!(outcome, Err(OracleError::CaseParse { .. })),
            "{case}: an unknown governance field fails parsing"
        );
        return;
    }
    assert_failure(case, &governance_error(outcome), expected);
}

const WITHOUT_DASHBOARD: &str = "name: governance-fixture\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ndatabase:\n  seed:\n    users: [{user_id: alice}]\n    dashboard_governance: {}\n";

fn without_dashboard() -> String {
    format!("{WITHOUT_DASHBOARD}{CASE_SUFFIX}")
}

fn rejected_cases() -> [(&'static str, String, Rejected); 7] {
    [
        (
            "missing_dashboard",
            without_dashboard(),
            Rejected::MissingDashboard,
        ),
        (
            "negative_counter",
            case_with(
                "    dashboard_governance:\n      metrics:\n        - {guardrail_id: gr-pii, date: '2026-01-04', requests_evaluated: -1}\n",
            ),
            Rejected::NegativeCounter,
        ),
        (
            "duplicate_identity",
            case_with(
                "    dashboard_governance:\n      tools:\n        - {tool_id: tool-same, tool_name: first}\n        - {tool_id: tool-same, tool_name: second}\n",
            ),
            Rejected::DuplicateIdentity,
        ),
        (
            "unknown_log",
            case_with(
                "    dashboard_governance:\n      guardrail_index:\n        - {request_id: req-missing, guardrail_id: gr-pii, start_time: '2026-01-04T10:00:00.000'}\n",
            ),
            Rejected::UnknownLog,
        ),
        (
            "unknown_key",
            case_with(
                "    dashboard_governance:\n      tools:\n        - {tool_id: tool-github, tool_name: mcp__github/search, key_index: 7}\n",
            ),
            Rejected::UnknownKey,
        ),
        (
            "missing_team",
            case_with(
                "    dashboard_governance:\n      usage_units:\n        - {guardrail_id: gr-pii, date: '2026-01-04', team_id: team-missing, usage_unit: textUnits, units: 1}\n",
            ),
            Rejected::MissingTeam,
        ),
        (
            "unknown_field",
            case_with("    dashboard_governance:\n      metrics: []\n      region: eu\n"),
            Rejected::Parse,
        ),
    ]
}

#[test]
fn governance_case_seed_rejects() {
    for (case, yaml, expected) in rejected_cases() {
        assert_rejection(
            case,
            decode_case(Path::new("<bytes>"), yaml.as_bytes()),
            expected,
        );
    }
}

fn corpus_paths() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_case");
    let mut paths = fs::read_dir(root)
        .expect("the http case corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("dashboard-governance-"))
        })
        .collect::<Vec<PathBuf>>();
    paths.sort();
    paths
}

fn corpus_expectation(name: &str) -> Rejected {
    match name {
        "dashboard-governance-without-dashboard.yaml" => Rejected::MissingDashboard,
        "dashboard-governance-unknown-field.yaml" => Rejected::Parse,
        "dashboard-governance-duplicate-metric.yaml" => Rejected::DuplicateIdentity,
        "dashboard-governance-unknown-key.yaml" => Rejected::UnknownKey,
        "dashboard-governance-unknown-log.yaml" => Rejected::UnknownLog,
        "dashboard-governance-negative-units.yaml" => Rejected::NegativeCounter,
        other => panic!("unexpected governance corpus seed {other}"),
    }
}

fn replay_seed(path: PathBuf) {
    let bytes = fs::read(&path).expect("the corpus seed reads");
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .expect("the corpus seed is named");
    assert_rejection(
        name,
        decode_case(Path::new("<bytes>"), &bytes),
        corpus_expectation(name),
    );
}

#[test]
fn decodes_governance_fixture_corpus() {
    let paths = corpus_paths();
    assert_eq!(
        paths.len(),
        6,
        "the governance fixture corpus carries its six seeds"
    );
    paths.into_iter().for_each(replay_seed);
}

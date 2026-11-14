#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle::OracleError;
use betterlite_oracle::case::{HttpCase, decode_case};
use betterlite_postgres_fixture::dashboard_governance::DashboardGovernanceError;
use betterlite_postgres_fixture::error::FixtureError;
use rstest::rstest;

use self::seed::{CASE_SUFFIX, case_with};

mod seed;

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

fn governance_error(outcome: Result<HttpCase, OracleError>) -> DashboardGovernanceError {
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

fn assert_rejection(case: &str, outcome: Result<HttpCase, OracleError>, expected: Rejected) {
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

#[rstest]
#[case::missing_dashboard(without_dashboard(), Rejected::MissingDashboard)]
#[case::negative_counter(
    case_with(
        "    dashboard_governance:\n      metrics:\n        - {guardrail_id: gr-pii, date: '2026-01-04', requests_evaluated: -1}\n"
    ),
    Rejected::NegativeCounter
)]
#[case::duplicate_identity(
    case_with(
        "    dashboard_governance:\n      tools:\n        - {tool_id: tool-same, tool_name: first}\n        - {tool_id: tool-same, tool_name: second}\n"
    ),
    Rejected::DuplicateIdentity
)]
#[case::unknown_log(
    case_with(
        "    dashboard_governance:\n      guardrail_index:\n        - {request_id: req-missing, guardrail_id: gr-pii, start_time: '2026-01-04T10:00:00.000'}\n"
    ),
    Rejected::UnknownLog
)]
#[case::unknown_key(
    case_with(
        "    dashboard_governance:\n      tools:\n        - {tool_id: tool-github, tool_name: mcp__github/search, key_index: 7}\n"
    ),
    Rejected::UnknownKey
)]
#[case::missing_team(
    case_with(
        "    dashboard_governance:\n      usage_units:\n        - {guardrail_id: gr-pii, date: '2026-01-04', team_id: team-missing, usage_unit: textUnits, units: 1}\n"
    ),
    Rejected::MissingTeam
)]
#[case::unknown_field(
    case_with("    dashboard_governance:\n      metrics: []\n      region: eu\n"),
    Rejected::Parse
)]
fn governance_case_seed_rejects(#[case] yaml: String, #[case] expected: Rejected) {
    assert_rejection(
        &yaml,
        decode_case(Path::new("<bytes>"), yaml.as_bytes()),
        expected,
    );
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

#[rstest]
fn decodes_governance_fixture_corpus(
    #[files("fuzz/corpus/http_case/dashboard-governance-*.yaml")] path: PathBuf,
) {
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

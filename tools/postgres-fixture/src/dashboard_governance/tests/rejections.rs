use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::NoTls;
use uuid::Uuid;

use crate::connection::PostgresFixture;
use crate::dashboard_governance::DashboardGovernanceError;
use crate::error::FixtureError;
use crate::seed::FixtureSeed;
use crate::tests::{FixedName, postgres_server};

use super::support::{
    boundary_governance, duplicate_guardrail_index, duplicate_metric, duplicate_tool_id,
    duplicate_tool_index, duplicate_tool_name, duplicate_usage_unit, failed_insert,
    governance_error, governance_without_dashboard, guardrail_index_over_limit,
    long_guardrail_policy, long_metric_date, long_metric_identity, long_origin, long_start_time,
    long_tool_identity, long_usage_unit_name, metrics_over_limit, negative_call_count,
    negative_requests, negative_units, negative_untracked_units, tool_index_over_limit,
    tools_over_limit, unknown_guardrail_log, unknown_tool_key, unknown_tool_log, unknown_tool_team,
    unknown_usage_key, unknown_usage_team, usage_units_over_limit,
};

#[derive(Debug, Clone, Copy)]
enum Failure {
    MissingDashboard,
    TooManyRows(&'static str),
    TextTooLong(&'static str, &'static str),
    DuplicateIdentity(&'static str),
    UnknownKey(&'static str),
    MissingTeam(&'static str),
    UnknownLog(&'static str),
    NegativeCounter(&'static str, &'static str),
    Insert(&'static str),
}

#[derive(Debug, Clone, Copy)]
enum Expectation {
    Accepted,
    Rejected(Failure),
    BeforeCreation(Failure),
    AfterCreation(Failure),
}

fn assert_failure(case: &str, error: &DashboardGovernanceError, failure: &Failure) {
    let matched = match (error, failure) {
        (DashboardGovernanceError::MissingDashboard, Failure::MissingDashboard) => true,
        (DashboardGovernanceError::TooManyRows { list, .. }, Failure::TooManyRows(expected))
        | (
            DashboardGovernanceError::DuplicateIdentity { list, .. },
            Failure::DuplicateIdentity(expected),
        )
        | (DashboardGovernanceError::UnknownKey { list, .. }, Failure::UnknownKey(expected))
        | (DashboardGovernanceError::MissingTeam { list, .. }, Failure::MissingTeam(expected))
        | (DashboardGovernanceError::UnknownLog { list, .. }, Failure::UnknownLog(expected))
        | (DashboardGovernanceError::Insert { list, .. }, Failure::Insert(expected)) => {
            list == expected
        }
        (
            DashboardGovernanceError::TextTooLong { list, field, .. },
            Failure::TextTooLong(expected_list, expected_field),
        )
        | (
            DashboardGovernanceError::NegativeCounter { list, field, .. },
            Failure::NegativeCounter(expected_list, expected_field),
        ) => list == expected_list && field == expected_field,
        _ => false,
    };
    assert!(matched, "{case}: unexpected governance error {error:?}");
}

fn unique_name(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

async fn database_exists(admin_url: &SecretString, name: &str) -> bool {
    let (client, connection) = tokio_postgres::connect(admin_url.expose_secret(), NoTls)
        .await
        .expect("the admin database accepts connections");
    let mut driver = JoinSet::new();
    driver.spawn(async move {
        connection.await.expect("the admin driver stays up");
    });
    let exists = client
        .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&name])
        .await
        .expect("the database catalog reads")
        .is_some();
    drop(client);
    driver
        .join_next()
        .await
        .expect("the admin driver joined")
        .expect("the admin driver finished");
    exists
}

async fn assert_provisioning_failure(case: &str, seed: &FixtureSeed, failure: &Failure) {
    let (_server, admin_url) = postgres_server().await;
    let name = unique_name("governance_rejected");
    let result = PostgresFixture::provision_with_name_generator(
        Some(&admin_url),
        seed,
        &mut FixedName(name.clone()),
    )
    .resolve()
    .await;
    let Err(FixtureError::DashboardGovernance(error)) = result else {
        panic!("{case}: expected a governance fixture failure");
    };
    assert_failure(case, &error, failure);
    assert!(
        !database_exists(&admin_url, &name).await,
        "{case}: a failed governance seed leaves no fixture database behind"
    );
}

fn rejection_cases() -> Vec<(&'static str, FixtureSeed, Expectation)> {
    vec![
        (
            "too_many_metrics",
            metrics_over_limit(),
            Expectation::Rejected(Failure::TooManyRows("metrics")),
        ),
        (
            "too_many_usage_units",
            usage_units_over_limit(),
            Expectation::Rejected(Failure::TooManyRows("usage_units")),
        ),
        (
            "too_many_guardrail_index",
            guardrail_index_over_limit(),
            Expectation::Rejected(Failure::TooManyRows("guardrail_index")),
        ),
        (
            "too_many_tool_index",
            tool_index_over_limit(),
            Expectation::Rejected(Failure::TooManyRows("tool_index")),
        ),
        (
            "too_many_tools",
            tools_over_limit(),
            Expectation::Rejected(Failure::TooManyRows("tools")),
        ),
        (
            "long_metric_identity",
            long_metric_identity(),
            Expectation::Rejected(Failure::TextTooLong("metrics", "guardrail_id")),
        ),
        (
            "long_metric_date",
            long_metric_date(),
            Expectation::Rejected(Failure::TextTooLong("metrics", "date")),
        ),
        (
            "long_tool_identity",
            long_tool_identity(),
            Expectation::Rejected(Failure::TextTooLong("tools", "tool_id")),
        ),
        (
            "long_tool_start_time",
            long_start_time(),
            Expectation::Rejected(Failure::TextTooLong("tool_index", "start_time")),
        ),
        (
            "long_tool_origin",
            long_origin(),
            Expectation::Rejected(Failure::TextTooLong("tools", "origin")),
        ),
        (
            "long_guardrail_policy",
            long_guardrail_policy(),
            Expectation::Rejected(Failure::TextTooLong("guardrail_index", "policy_id")),
        ),
        (
            "long_usage_unit_name",
            long_usage_unit_name(),
            Expectation::Rejected(Failure::TextTooLong("usage_units", "usage_unit")),
        ),
        (
            "negative_requests",
            negative_requests(),
            Expectation::Rejected(Failure::NegativeCounter("metrics", "requests_evaluated")),
        ),
        (
            "negative_units",
            negative_units(),
            Expectation::Rejected(Failure::NegativeCounter("usage_units", "units")),
        ),
        (
            "negative_untracked_units",
            negative_untracked_units(),
            Expectation::Rejected(Failure::NegativeCounter("usage_units", "untracked_units")),
        ),
        (
            "negative_call_count",
            negative_call_count(),
            Expectation::Rejected(Failure::NegativeCounter("tools", "call_count")),
        ),
        (
            "duplicate_metric",
            duplicate_metric(),
            Expectation::Rejected(Failure::DuplicateIdentity("metrics")),
        ),
        (
            "duplicate_usage_unit",
            duplicate_usage_unit(),
            Expectation::Rejected(Failure::DuplicateIdentity("usage_units")),
        ),
        (
            "duplicate_tool_id",
            duplicate_tool_id(),
            Expectation::Rejected(Failure::DuplicateIdentity("tools")),
        ),
        (
            "duplicate_tool_name",
            duplicate_tool_name(),
            Expectation::Rejected(Failure::DuplicateIdentity("tools")),
        ),
        (
            "duplicate_guardrail_index",
            duplicate_guardrail_index(),
            Expectation::Rejected(Failure::DuplicateIdentity("guardrail_index")),
        ),
        (
            "duplicate_tool_index",
            duplicate_tool_index(),
            Expectation::Rejected(Failure::DuplicateIdentity("tool_index")),
        ),
        (
            "unknown_tool_key",
            unknown_tool_key(),
            Expectation::Rejected(Failure::UnknownKey("tools")),
        ),
        (
            "unknown_usage_key",
            unknown_usage_key(),
            Expectation::Rejected(Failure::UnknownKey("usage_units")),
        ),
        (
            "unknown_tool_team",
            unknown_tool_team(),
            Expectation::Rejected(Failure::MissingTeam("tools")),
        ),
        (
            "unknown_usage_team",
            unknown_usage_team(),
            Expectation::Rejected(Failure::MissingTeam("usage_units")),
        ),
        (
            "unknown_guardrail_log",
            unknown_guardrail_log(),
            Expectation::Rejected(Failure::UnknownLog("guardrail_index")),
        ),
        (
            "unknown_tool_log",
            unknown_tool_log(),
            Expectation::Rejected(Failure::UnknownLog("tool_index")),
        ),
        (
            "boundary_accepted",
            boundary_governance(),
            Expectation::Accepted,
        ),
        (
            "missing_dashboard",
            governance_without_dashboard(),
            Expectation::Rejected(Failure::MissingDashboard),
        ),
        (
            "rejected_before_creation",
            governance_without_dashboard(),
            Expectation::BeforeCreation(Failure::MissingDashboard),
        ),
        (
            "cleaned_up_after_failed_insert",
            failed_insert(),
            Expectation::AfterCreation(Failure::Insert("tool_index")),
        ),
    ]
}

#[tokio::test]
async fn governance_seed_rejections() {
    for (case, seed, expectation) in rejection_cases() {
        match expectation {
            Expectation::Accepted => {
                if let Err(error) = seed.validate() {
                    panic!("{case} should validate, got {error}");
                }
            }
            Expectation::Rejected(failure) => {
                assert_failure(case, &governance_error(&seed), &failure);
            }
            Expectation::BeforeCreation(failure) | Expectation::AfterCreation(failure) => {
                assert_provisioning_failure(case, &seed, &failure).await;
            }
        }
    }
}

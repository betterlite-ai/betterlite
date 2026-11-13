use rstest::rstest;
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

fn assert_failure(error: &DashboardGovernanceError, failure: &Failure) {
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
    assert!(matched, "unexpected governance error {error:?}");
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

async fn assert_provisioning_failure(seed: &FixtureSeed, failure: &Failure) {
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
        panic!("expected a governance fixture failure");
    };
    assert_failure(&error, failure);
    assert!(
        !database_exists(&admin_url, &name).await,
        "a failed governance seed leaves no fixture database behind"
    );
}

#[rstest]
#[case::too_many_metrics(
    metrics_over_limit(),
    Expectation::Rejected(Failure::TooManyRows("metrics"))
)]
#[case::too_many_usage_units(
    usage_units_over_limit(),
    Expectation::Rejected(Failure::TooManyRows("usage_units"))
)]
#[case::too_many_guardrail_index(
    guardrail_index_over_limit(),
    Expectation::Rejected(Failure::TooManyRows("guardrail_index"))
)]
#[case::too_many_tool_index(
    tool_index_over_limit(),
    Expectation::Rejected(Failure::TooManyRows("tool_index"))
)]
#[case::too_many_tools(
    tools_over_limit(),
    Expectation::Rejected(Failure::TooManyRows("tools"))
)]
#[case::long_metric_identity(
    long_metric_identity(),
    Expectation::Rejected(Failure::TextTooLong("metrics", "guardrail_id"))
)]
#[case::long_metric_date(
    long_metric_date(),
    Expectation::Rejected(Failure::TextTooLong("metrics", "date"))
)]
#[case::long_tool_identity(
    long_tool_identity(),
    Expectation::Rejected(Failure::TextTooLong("tools", "tool_id"))
)]
#[case::long_tool_start_time(
    long_start_time(),
    Expectation::Rejected(Failure::TextTooLong("tool_index", "start_time"))
)]
#[case::long_tool_origin(
    long_origin(),
    Expectation::Rejected(Failure::TextTooLong("tools", "origin"))
)]
#[case::long_guardrail_policy(
    long_guardrail_policy(),
    Expectation::Rejected(Failure::TextTooLong("guardrail_index", "policy_id"))
)]
#[case::long_usage_unit_name(
    long_usage_unit_name(),
    Expectation::Rejected(Failure::TextTooLong("usage_units", "usage_unit"))
)]
#[case::negative_requests(
    negative_requests(),
    Expectation::Rejected(Failure::NegativeCounter("metrics", "requests_evaluated"))
)]
#[case::negative_units(
    negative_units(),
    Expectation::Rejected(Failure::NegativeCounter("usage_units", "units"))
)]
#[case::negative_untracked_units(
    negative_untracked_units(),
    Expectation::Rejected(Failure::NegativeCounter("usage_units", "untracked_units"))
)]
#[case::negative_call_count(
    negative_call_count(),
    Expectation::Rejected(Failure::NegativeCounter("tools", "call_count"))
)]
#[case::duplicate_metric(
    duplicate_metric(),
    Expectation::Rejected(Failure::DuplicateIdentity("metrics"))
)]
#[case::duplicate_usage_unit(
    duplicate_usage_unit(),
    Expectation::Rejected(Failure::DuplicateIdentity("usage_units"))
)]
#[case::duplicate_tool_id(
    duplicate_tool_id(),
    Expectation::Rejected(Failure::DuplicateIdentity("tools"))
)]
#[case::duplicate_tool_name(
    duplicate_tool_name(),
    Expectation::Rejected(Failure::DuplicateIdentity("tools"))
)]
#[case::duplicate_guardrail_index(
    duplicate_guardrail_index(),
    Expectation::Rejected(Failure::DuplicateIdentity("guardrail_index"))
)]
#[case::duplicate_tool_index(
    duplicate_tool_index(),
    Expectation::Rejected(Failure::DuplicateIdentity("tool_index"))
)]
#[case::unknown_tool_key(
    unknown_tool_key(),
    Expectation::Rejected(Failure::UnknownKey("tools"))
)]
#[case::unknown_usage_key(
    unknown_usage_key(),
    Expectation::Rejected(Failure::UnknownKey("usage_units"))
)]
#[case::unknown_tool_team(
    unknown_tool_team(),
    Expectation::Rejected(Failure::MissingTeam("tools"))
)]
#[case::unknown_usage_team(
    unknown_usage_team(),
    Expectation::Rejected(Failure::MissingTeam("usage_units"))
)]
#[case::unknown_guardrail_log(
    unknown_guardrail_log(),
    Expectation::Rejected(Failure::UnknownLog("guardrail_index"))
)]
#[case::unknown_tool_log(
    unknown_tool_log(),
    Expectation::Rejected(Failure::UnknownLog("tool_index"))
)]
#[case::boundary_accepted(boundary_governance(), Expectation::Accepted)]
#[case::missing_dashboard(
    governance_without_dashboard(),
    Expectation::Rejected(Failure::MissingDashboard)
)]
#[case::rejected_before_creation(
    governance_without_dashboard(),
    Expectation::BeforeCreation(Failure::MissingDashboard)
)]
#[case::cleaned_up_after_failed_insert(
    failed_insert(),
    Expectation::AfterCreation(Failure::Insert("tool_index"))
)]
#[tokio::test]
async fn governance_seed_rejections(#[case] seed: FixtureSeed, #[case] expectation: Expectation) {
    match expectation {
        Expectation::Accepted => seed.validate().unwrap(),
        Expectation::Rejected(failure) => assert_failure(&governance_error(&seed), &failure),
        Expectation::BeforeCreation(failure) | Expectation::AfterCreation(failure) => {
            assert_provisioning_failure(&seed, &failure).await;
        }
    }
}

#![cfg(test)]

use betterlite_policy::registry::{PolicyRegistry, PolicySource};
use betterlite_policy::values::{Policy, PolicyGuardrails};
use rstest::rstest;
use tracing::Level;

use super::delete_policy_from_db;
use crate::policy_persistence::Deletion;
use crate::policy_persistence::tests::{Call, CapturedLogs, Fixture, id_filter, policy_row};

const NONE_PROMOTED: &str = "Production version was deleted. No other version was promoted. Promote another version to production if this policy should remain active.";

fn described(description: &str) -> Policy {
    let guardrails = PolicyGuardrails::default();
    Policy {
        inherit: None,
        description: Some(description.to_owned()),
        guardrails,
        condition: None,
        pipeline: None,
    }
}

fn fixture(status: Option<&str>, config: bool) -> Fixture {
    let mut row = policy_row("p1", "pii", "draft", "2026-01-01T00:00:00Z");
    row.version_status = status.map(str::to_owned);
    let fixture = Fixture::new(&[row], &[]);
    let mut registry = PolicyRegistry::default();
    if config {
        registry.add_policy("pii", described("config"), PolicySource::Config);
    }
    registry.add_policy("pii", described("db"), PolicySource::Db);
    *fixture.policies.lock().unwrap() = registry;
    fixture
}

#[rstest]
#[case::draft("draft")]
#[case::published("published")]
#[tokio::test]
async fn leaves_the_registry_alone_for_a_non_production_version(#[case] status: &str) {
    let fixture = fixture(Some(status), false);
    let deleted = delete_policy_from_db(&fixture.policy_store(), "p1")
        .await
        .unwrap();
    assert_eq!(
        deleted,
        Deletion {
            message: "Policy p1 deleted successfully".to_owned(),
            warning: None
        }
    );
    assert_eq!(
        fixture.client.policies.calls().last(),
        Some(&Call::Delete(id_filter("policy_id", "p1")))
    );
    assert!(fixture.client.policies.rows().is_empty());
    assert_eq!(
        fixture.policies.into_inner().unwrap().policies.get("pii"),
        Some(&described("db"))
    );
}

#[rstest]
#[case::explicit_without_config(Some("production"), false, NONE_PROMOTED, None)]
#[case::missing_status_without_config(None, false, NONE_PROMOTED, None)]
#[case::config_fallback(
    Some("production"),
    true,
    "Production version was deleted. The config-defined policy with the same name is active again.",
    Some(PolicySource::Config)
)]
#[tokio::test]
async fn removes_a_deleted_production_version_and_warns(
    #[case] status: Option<&str>,
    #[case] config: bool,
    #[case] warning: &str,
    #[case] source: Option<PolicySource>,
) {
    let fixture = fixture(status, config);
    let deleted = delete_policy_from_db(&fixture.policy_store(), "p1")
        .await
        .unwrap();
    assert_eq!(
        (deleted.message.as_str(), deleted.warning.as_deref()),
        ("Policy p1 deleted successfully", Some(warning))
    );
    let registry = fixture.policies.into_inner().unwrap();
    assert_eq!(registry.sources.get("pii").copied(), source);
    assert_eq!(
        registry.policies.get("pii"),
        source.map(|_| described("config")).as_ref()
    );
}

#[rstest]
#[case::unknown_id(
    "nope",
    None,
    "Error deleting policy from DB: Policy with ID nope not found"
)]
#[case::failed_delete(
    "p1",
    Some("delete"),
    "Error deleting policy from DB: table refused delete"
)]
#[tokio::test]
async fn reports_failures_and_keeps_the_registry(
    #[case] policy_id: &str,
    #[case] failing: Option<&'static str>,
    #[case] message: &str,
) {
    let (logs, _guard) = CapturedLogs::install();
    let fixture = fixture(Some("production"), false);
    if let Some(method) = failing {
        fixture.client.policies.fail(method);
    }
    let error = delete_policy_from_db(&fixture.policy_store(), policy_id)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
    assert_eq!(logs.at(Level::ERROR), [message]);
    assert!(
        fixture
            .policies
            .into_inner()
            .unwrap()
            .policies
            .contains_key("pii")
    );
}

#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_policy::registry::PolicySource;
use betterlite_policy::row_adapter::{PolicyClause, PolicyOrder, PolicySort};
use betterlite_policy::values::{Policy, PolicyGuardrails};
use tracing::Level;

use super::sync_policies_from_db;
use crate::policy_persistence::tests::{
    Call, CapturedLogs, Fixture, id_filter, object, policy_row,
};

const VERSIONS_FILTER: &str = r#"{"version_status": {"in": ["draft", "published"]}}"#;

fn guarded(guardrail: &str) -> Policy {
    let guardrails = PolicyGuardrails {
        add: Some(vec![guardrail.to_owned()]),
        remove: Some(Vec::new()),
    };
    Policy {
        inherit: None,
        description: None,
        guardrails,
        condition: None,
        pipeline: None,
    }
}

fn fixture(extra: Option<&str>) -> Fixture {
    let mut production = policy_row("p1", "pii", "production", "2026-01-01T00:00:00Z");
    production.condition = Some(object(&BTreeMap::from([("model", "gpt-.*")])));
    let mut rows = vec![
        production,
        policy_row("p2", "pii", "draft", "2026-01-02T00:00:00Z"),
        policy_row("p3", "other", "published", "2026-01-03T00:00:00Z"),
        policy_row("p4", "base", "production", "2026-01-04T00:00:00Z"),
    ];
    if let Some(broken) = extra {
        let mut row = policy_row("p9", broken, "draft", "2026-01-05T00:00:00Z");
        row.pipeline = Some(object(&BTreeMap::from([("steps", Vec::<String>::new())])));
        rows.push(row);
    }
    let fixture = Fixture::new(&rows, &[]);
    let mut registry = fixture.policies.lock().unwrap();
    registry.add_policy("pii", guarded("config-pii"), PolicySource::Config);
    registry.add_policy("keep", guarded("config-keep"), PolicySource::Config);
    registry.add_policy("stale", guarded("stale"), PolicySource::Db);
    registry
        .policies_by_id
        .insert("gone".to_owned(), ("stale".to_owned(), guarded("stale")));
    registry.initialized = false;
    drop(registry);
    fixture
}

#[tokio::test]
async fn loads_production_by_name_and_other_versions_by_id() {
    let fixture = fixture(None);
    sync_policies_from_db(&fixture.policy_store())
        .await
        .unwrap();
    let registry = fixture.policies.into_inner().unwrap();
    let mut pii = guarded("pii-guard");
    pii.condition = serde_json::from_str(r#"{"model": "gpt-.*"}"#).unwrap();
    let policies = [
        ("base", guarded("base-guard")),
        ("keep", guarded("config-keep")),
        ("pii", pii),
    ];
    assert_eq!(
        registry.policies,
        BTreeMap::from(policies.map(|(name, policy)| (name.to_owned(), policy)))
    );
    let sources = [
        ("base", PolicySource::Db),
        ("keep", PolicySource::Config),
        ("pii", PolicySource::Db),
    ];
    assert_eq!(
        registry.sources,
        BTreeMap::from(sources.map(|(name, source)| (name.to_owned(), source)))
    );
    let by_id = [("p2", "pii"), ("p3", "other")].map(|(id, name)| {
        (
            id.to_owned(),
            (name.to_owned(), guarded(&format!("{name}-guard"))),
        )
    });
    assert_eq!(registry.policies_by_id, BTreeMap::from(by_id));
    assert_eq!(
        (registry.config_policies.len(), registry.initialized),
        (2, true)
    );
}

#[tokio::test]
async fn reads_production_then_other_versions_newest_first_and_logs_counts() {
    let (logs, _guard) = CapturedLogs::install();
    let fixture = fixture(None);
    sync_policies_from_db(&fixture.policy_store())
        .await
        .unwrap();
    let warning =
        "Policy 'pii' is defined in both config.yaml and the DB; the DB version takes precedence";
    assert_eq!(logs.at(Level::WARN), [warning]);
    let info = "Synced 2 production policies and 2 draft/published (by ID) from DB to in-memory registry (2 config-defined policies preserved)";
    assert_eq!(logs.at(Level::INFO), [info]);
    let order = || {
        Some(PolicyOrder::from([(
            "created_at".to_owned(),
            PolicySort::Desc,
        )]))
    };
    let versions: PolicyClause = serde_json::from_str(VERSIONS_FILTER).unwrap();
    let production = Call::FindMany(Some(id_filter("version_status", "production")), order());
    assert_eq!(
        fixture.client.policies.calls(),
        [production, Call::FindMany(Some(versions), order())]
    );
}

#[tokio::test]
async fn a_failed_listing_names_both_operations_and_changes_nothing() {
    let fixture = fixture(None);
    fixture.client.policies.fail("find_many");
    let error = sync_policies_from_db(&fixture.policy_store())
        .await
        .unwrap_err();
    let message =
        "Error syncing policies from DB: Error getting policies from DB: table refused find_many";
    assert_eq!(error.to_string(), message);
    let registry = fixture.policies.into_inner().unwrap();
    assert_eq!(
        (
            registry.policies.contains_key("stale"),
            registry.initialized
        ),
        (true, false)
    );
}

#[tokio::test]
async fn keeps_the_published_production_map_when_a_stored_version_is_invalid() {
    let fixture = fixture(Some("broken"));
    let error = sync_policies_from_db(&fixture.policy_store())
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Error syncing policies from DB: Invalid policy 'broken'")
    );
    let registry = fixture.policies.into_inner().unwrap();
    let names: Vec<&str> = registry.policies.keys().map(String::as_str).collect();
    assert_eq!(
        (names, registry.policies_by_id.len(), registry.initialized),
        (vec!["base", "keep", "pii"], 0, false)
    );
}

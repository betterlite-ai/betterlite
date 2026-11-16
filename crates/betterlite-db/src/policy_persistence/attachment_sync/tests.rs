#![cfg(test)]

use betterlite_policy::values::PolicyAttachment;
use tracing::Level;

use super::sync_attachments_from_db;
use crate::policy_persistence::tests::{CapturedLogs, Fixture, attachment_row};

const SCOPED: &str = r#"{"policy": "scoped", "scope": null, "teams": ["team-a"], "keys": ["key-a"], "models": ["gpt-4"], "tags": ["prod"], "priority": 5, "default": true}"#;

fn global(name: &str) -> PolicyAttachment {
    let scope = Some("*".to_owned());
    let policy = name.to_owned();
    PolicyAttachment {
        policy,
        scope,
        teams: None,
        keys: None,
        models: None,
        tags: None,
        priority: None,
        default: false,
    }
}

fn fixture() -> Fixture {
    let mut scoped = attachment_row("a2", "scoped", "2026-01-02T00:00:00Z");
    scoped.keys = vec!["key-a".to_owned()];
    scoped.models = vec!["gpt-4".to_owned()];
    scoped.tags = vec!["prod".to_owned()];
    scoped.priority = Some(5_i32);
    scoped.is_default = true;
    let mut bare = attachment_row("a1", "bare", "2026-01-01T00:00:00Z");
    bare.teams = Vec::new();
    bare.scope = Some("*".to_owned());
    let fixture = Fixture::new(&[], &[bare, scoped]);
    let mut loaded = fixture.attachments.lock().unwrap();
    loaded.load_attachments(vec![global("config")]);
    loaded.add_attachment(global("stale"));
    loaded.initialized = false;
    drop(loaded);
    fixture
}

#[tokio::test]
async fn puts_config_attachments_first_then_the_table_newest_first() {
    let fixture = fixture();
    let (logs, _guard) = CapturedLogs::install();
    sync_attachments_from_db(&fixture.attachment_store())
        .await
        .unwrap();
    let registry = fixture.attachments.into_inner().unwrap();
    let scoped: PolicyAttachment = serde_json::from_str(SCOPED).unwrap();
    assert_eq!(
        registry.attachments,
        [global("config"), scoped, global("bare")]
    );
    assert_eq!(
        (registry.config_attachments, registry.initialized),
        (vec![global("config")], true)
    );
    let info = "Synced 2 attachments from DB to in-memory registry (1 config-defined attachments preserved)";
    assert_eq!(logs.at(Level::INFO), [info]);
}

#[tokio::test]
async fn a_failed_listing_names_both_operations_and_changes_nothing() {
    let fixture = fixture();
    fixture.client.attachments.fail("find_many");
    let error = sync_attachments_from_db(&fixture.attachment_store())
        .await
        .unwrap_err();
    let message = "Error syncing attachments from DB: Error getting attachments from DB: table refused find_many";
    assert_eq!(error.to_string(), message);
    let registry = fixture.attachments.into_inner().unwrap();
    assert_eq!(
        (registry.attachments, registry.initialized),
        (vec![global("config"), global("stale")], false)
    );
}

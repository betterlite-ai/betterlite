#![cfg(test)]

mod postgres;
mod support;

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Utc};
use deadpool_postgres::Client;

use crate::database_faults::DbError;
use crate::database_faults::reconnect::DbReconnect;
use crate::postgres_store::session::PostgresStore;
use crate::tool_records::decode::decode_tool_record;
use crate::tool_records::records::{ToolInputPolicy, ToolOutputPolicy};

use self::support::captured_logs;
use super::blockers::{
    add_tool_to_object_permission_blocked, remove_tool_from_object_permission_blocked,
};
use super::discovery::batch_upsert_tools;
use super::policy_update::update_tool_policy;
use super::records::{BlockedToolRow, ToolDiscoveryItem, ToolPolicyUpdate};
use super::refresh::{ToolPolicyStore, load_blocked_tools, sync_tool_policy_from_db};
use super::registry::{ToolPolicyRegistry, blocked_tools_map, tool_policy_maps};

#[derive(Default)]
pub(crate) struct Reconnect {
    reasons: Mutex<Vec<String>>,
}

impl Reconnect {
    pub(crate) fn reasons(&self) -> Vec<String> {
        self.reasons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl DbReconnect for Reconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        _timeout_seconds: f64,
        _lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        self.reasons
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(reason.to_owned());
        Box::pin(ready(Ok(false)))
    }
}

fn instant() -> DateTime<Utc> {
    DateTime::from_timestamp(1_772_000_000, 0).expect("a test instant")
}

fn item(tool_name: Option<&str>) -> ToolDiscoveryItem {
    ToolDiscoveryItem {
        tool_name: tool_name.map(str::to_owned),
        origin: None,
        created_by: None,
        key_hash: None,
        team_id: None,
        key_alias: None,
        user_agent: None,
    }
}

fn registry() -> ToolPolicyRegistry {
    ToolPolicyRegistry {
        input_policies: BTreeMap::from([
            ("blocked-tool".to_owned(), ToolInputPolicy::Trusted),
            ("search".to_owned(), ToolInputPolicy::Trusted),
        ]),
        output_policies: BTreeMap::from([
            ("blocked-tool".to_owned(), ToolOutputPolicy::Trusted),
            ("search".to_owned(), ToolOutputPolicy::Trusted),
        ]),
        blocked_tools_by_permission: BTreeMap::from([
            ("key-perm".to_owned(), vec!["blocked-tool".to_owned()]),
            ("team-perm".to_owned(), vec!["team-tool".to_owned()]),
        ]),
        initialized: true,
    }
}

#[test]
fn effective_policies_union_the_key_and_team_blocked_lists() {
    let policies = registry().effective_policies(
        &[
            "search".to_owned(),
            "blocked-tool".to_owned(),
            "team-tool".to_owned(),
        ],
        Some("key-perm"),
        Some("team-perm"),
    );
    assert_eq!(
        policies,
        BTreeMap::from([
            ("blocked-tool".to_owned(), ToolInputPolicy::Blocked),
            ("search".to_owned(), ToolInputPolicy::Trusted),
            ("team-tool".to_owned(), ToolInputPolicy::Blocked),
        ])
    );
}

#[test]
fn effective_policies_trim_the_permission_ids() {
    let policies =
        registry().effective_policies(&["blocked-tool".to_owned()], Some(" key-perm "), None);
    assert_eq!(
        policies.get("blocked-tool"),
        Some(&ToolInputPolicy::Blocked)
    );
}

#[test]
fn effective_policies_default_unknown_tools_to_untrusted() {
    let policies = registry().effective_policies(&["missing".to_owned()], None, None);
    assert_eq!(policies.get("missing"), Some(&ToolInputPolicy::Untrusted));
}

#[test]
fn effective_policies_answer_nothing_without_tool_names() {
    assert!(
        registry()
            .effective_policies(&[], Some("key-perm"), None)
            .is_empty()
    );
}

#[test]
fn output_policy_ignores_input_policy_and_blocking() {
    let registry = registry();
    assert_eq!(
        registry.output_policy("blocked-tool"),
        ToolOutputPolicy::Trusted
    );
    assert_eq!(
        registry.input_policy("blocked-tool"),
        ToolInputPolicy::Trusted
    );
    assert_eq!(
        registry.output_policy("missing"),
        ToolOutputPolicy::Untrusted
    );
    assert_eq!(registry.input_policy("missing"), ToolInputPolicy::Untrusted);
}

#[test]
fn tool_policy_maps_keep_the_last_row_for_a_name() {
    let trusted = decode_tool_record(
        br#"{"tool_name":"search","input_policy":"trusted","output_policy":"trusted"}"#,
    )
    .expect("a trusted row");
    let untrusted = decode_tool_record(
        br#"{"tool_name":"search","input_policy":"untrusted","output_policy":"untrusted"}"#,
    )
    .expect("an untrusted row");
    let maps = tool_policy_maps(&[trusted, untrusted]);
    assert_eq!(
        maps.input_policies.get("search"),
        Some(&ToolInputPolicy::Untrusted)
    );
    assert_eq!(
        maps.output_policies.get("search"),
        Some(&ToolOutputPolicy::Untrusted)
    );
}

#[test]
fn blocked_tools_map_skips_empty_permission_ids_and_reads_null_as_empty() {
    let rows = [
        BlockedToolRow {
            object_permission_id: String::new(),
            blocked_tools: Some(vec!["ignored".to_owned()]),
        },
        BlockedToolRow {
            object_permission_id: "perm-null".to_owned(),
            blocked_tools: None,
        },
        BlockedToolRow {
            object_permission_id: "perm-list".to_owned(),
            blocked_tools: Some(vec!["search".to_owned()]),
        },
    ];
    let blocked = blocked_tools_map(&rows);
    assert_eq!(blocked.len(), 2);
    assert_eq!(blocked.get("perm-null"), Some(&Vec::new()));
    assert_eq!(blocked.get("perm-list"), Some(&vec!["search".to_owned()]));
}

#[tokio::test]
async fn discovery_without_a_database_logs_an_error() {
    let (logs, guard) = captured_logs();
    batch_upsert_tools(None::<&Client>, &[item(Some("search"))], instant()).await;
    drop(guard);
    assert!(
        logs.text()
            .contains("tool_registry_writer batch_upsert_tools error: no database connection")
    );
}

#[tokio::test]
async fn discovery_skips_a_batch_that_names_no_tool() {
    let (logs, guard) = captured_logs();
    batch_upsert_tools(None::<&Client>, &[item(None), item(Some(""))], instant()).await;
    drop(guard);
    assert!(!logs.text().contains("batch_upsert_tools"));
}

#[tokio::test]
async fn policy_update_without_a_database_returns_none() {
    let (logs, guard) = captured_logs();
    let update = ToolPolicyUpdate {
        tool_name: "search",
        updated_by: None,
        input_policy: None,
        output_policy: None,
    };
    assert!(
        update_tool_policy(None::<&Client>, &update, instant())
            .await
            .is_none()
    );
    drop(guard);
    assert!(
        logs.text()
            .contains("tool_registry_writer update_tool_policy error: no database connection")
    );
}

#[tokio::test]
async fn blocked_list_writes_without_a_database_return_false() {
    let (logs, guard) = captured_logs();
    assert!(!add_tool_to_object_permission_blocked(None::<&Client>, "perm-1", "search").await);
    assert!(!remove_tool_from_object_permission_blocked(None::<&Client>, "perm-1", "search").await);
    drop(guard);
    let text = logs.text();
    assert!(text.contains("add_tool_to_object_permission_blocked error: no database connection"));
    assert!(
        text.contains("remove_tool_from_object_permission_blocked error: no database connection")
    );
}

#[tokio::test]
async fn blocked_list_writes_with_empty_arguments_answer_false_before_the_database() {
    let (logs, guard) = captured_logs();
    assert!(!add_tool_to_object_permission_blocked(None::<&Client>, "", "search").await);
    assert!(!add_tool_to_object_permission_blocked(None::<&Client>, "perm-1", "").await);
    assert!(!remove_tool_from_object_permission_blocked(None::<&Client>, "", "search").await);
    assert!(!remove_tool_from_object_permission_blocked(None::<&Client>, "perm-1", "").await);
    drop(guard);
    assert!(!logs.text().contains("error"));
}

#[tokio::test]
async fn refresh_reconnects_a_missing_database_with_the_tools_reason() {
    let database = PostgresStore { pool: None };
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let store = ToolPolicyStore {
        database: &database,
        registry: &registry,
    };
    let mut reconnect = Reconnect::default();
    let result = sync_tool_policy_from_db(&store, Some(&mut reconnect)).await;
    assert!(result.is_err());
    assert_eq!(
        reconnect.reasons(),
        ["sync_tool_policy_from_db_tools_lookup_failure".to_owned()]
    );
    assert!(
        !registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .initialized
    );
}

#[tokio::test]
async fn permission_reads_reconnect_with_the_permissions_reason() {
    let database = PostgresStore { pool: None };
    let mut reconnect = Reconnect::default();
    let result = load_blocked_tools(&database, Some(&mut reconnect)).await;
    assert!(result.is_err());
    assert_eq!(
        reconnect.reasons(),
        ["sync_tool_policy_from_db_perms_lookup_failure".to_owned()]
    );
}

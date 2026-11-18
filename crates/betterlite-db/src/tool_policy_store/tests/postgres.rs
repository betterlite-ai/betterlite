use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use deadpool_postgres::Client;
use secrecy::ExposeSecret as _;
use tokio::task::yield_now;
use tokio_postgres::NoTls;
use tokio_postgres::error::SqlState;

use crate::budget_permission_rows::Timestamp;
use crate::database_faults::ErrorType;
use crate::postgres_store::errors::DbError as NativeDbError;
use crate::tool_policy_store::blockers::{BlockedChange, change_blocked_tools};
use crate::tool_policy_store::deadline::{StatementBounds, TlsCanceller};
use crate::tool_policy_store::discovery::batch_upsert_tools;
use crate::tool_policy_store::errors::ToolPolicyStoreError;
use crate::tool_policy_store::policy_update::update_tool_policy;
use crate::tool_policy_store::records::{ToolDiscoveryItem, ToolPolicyUpdate, UuidToolIdGenerator};
use crate::tool_policy_store::refresh::{ToolPolicyStore, sync_tool_policy_from_db};
use crate::tool_policy_store::registry::ToolPolicyRegistry;
use crate::tool_records::queries::get_tool;
use crate::tool_records::records::{LiteLLMToolTableRow, ToolInputPolicy, ToolOutputPolicy};

use super::support::{Db, TlsFront, captured_logs};
use super::{FixedIds, Reconnect, instant, item};

const DEADLINE: Duration = Duration::from_secs(5);

const POISON_TOOL_TRIGGER: &str = r#"CREATE FUNCTION reject_poison_tool() RETURNS trigger AS $$
BEGIN
    IF NEW."tool_name" = 'poison' THEN
        RAISE EXCEPTION 'poison tool';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER reject_poison_tool BEFORE INSERT OR UPDATE ON "LiteLLM_ToolTable"
    FOR EACH ROW EXECUTE FUNCTION reject_poison_tool();"#;

const LOCKED_PERMISSION_TRIGGER: &str = r#"CREATE FUNCTION reject_locked_permission() RETURNS trigger AS $$
BEGIN
    IF NEW."object_permission_id" IN ('locked-add', 'locked-remove') THEN
        RAISE EXCEPTION 'locked permission';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER reject_locked_permission BEFORE UPDATE ON "LiteLLM_ObjectPermissionTable"
    FOR EACH ROW EXECUTE FUNCTION reject_locked_permission();"#;

const TABLE_LOCK: &str = "BEGIN; LOCK TABLE \"LiteLLM_ToolTable\" IN ACCESS EXCLUSIVE MODE";

async fn stored_tool(db: &Db, tool_name: &str) -> LiteLLMToolTableRow {
    let client = db.client().await;
    get_tool(Some(&client), tool_name)
        .await
        .expect("the stored tool")
}

async fn blocked_list(db: &Db, object_permission_id: &str) -> Option<Vec<String>> {
    let client = db.client().await;
    let row = client
        .query_opt(
            "SELECT \"blocked_tools\" FROM \"LiteLLM_ObjectPermissionTable\" WHERE \"object_permission_id\" = $1",
            &[&object_permission_id],
        )
        .await
        .expect("the permission row");
    row.map(|row| row.try_get("blocked_tools").expect("the blocked list"))
}

async fn lock_waiters(db: &Db) -> i64 {
    let client = db.client().await;
    client
        .query_one(
            "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND state = 'active' AND query LIKE '%LiteLLM_ToolTable%'",
            &[],
        )
        .await
        .expect("the database activity")
        .get(0)
}

async fn wait_until_no_lock_waiters(db: &Db) -> i64 {
    let mut waiting = lock_waiters(db).await;
    for _ in 0..100 {
        if waiting == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        waiting = lock_waiters(db).await;
    }
    waiting
}

async fn sync(db: &Db, registry: &Mutex<ToolPolicyRegistry>) -> Result<(), ToolPolicyStoreError> {
    let store = ToolPolicyStore {
        database: &db.store,
        registry,
        bounds: StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
    };
    sync_tool_policy_from_db(&store, None::<&mut Reconnect>).await
}

fn trusted(name: &str) -> ToolPolicyUpdate<'_> {
    ToolPolicyUpdate {
        tool_name: name,
        updated_by: None,
        input_policy: Some(ToolInputPolicy::Trusted),
        output_policy: Some(ToolOutputPolicy::Trusted),
    }
}

async fn store_trusted(db: &Db, tool_name: &str, now: DateTime<Utc>) -> LiteLLMToolTableRow {
    update_tool_policy(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &trusted(tool_name),
        now,
    )
    .await
    .expect("the stored tool")
}

async fn check_blocked_change(
    client: Client,
    object_permission_id: &str,
    tool_name: &str,
    change: BlockedChange,
    expected: bool,
) {
    let changed = change_blocked_tools(
        Some(client),
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        object_permission_id,
        tool_name,
        change,
    )
    .await;
    assert_eq!(
        changed, expected,
        "{change:?} {tool_name} on {object_permission_id}"
    );
}

fn identified_item(
    tool_name: &str,
    origin: &str,
    created_by: &str,
    key_hash: &str,
    team_id: Option<&str>,
) -> ToolDiscoveryItem {
    ToolDiscoveryItem {
        tool_name: Some(tool_name.to_owned()),
        origin: Some(origin.to_owned()),
        created_by: Some(created_by.to_owned()),
        key_hash: Some(key_hash.to_owned()),
        team_id: team_id.map(str::to_owned),
        key_alias: None,
        user_agent: None,
    }
}

fn later_instant() -> DateTime<Utc> {
    DateTime::from_timestamp(1_772_000_100, 0).expect("a later instant")
}

#[tokio::test]
async fn discovery_creates_an_untrusted_tool_with_one_call() {
    let db = Db::start().await;
    let now = instant();
    let discovered = ToolDiscoveryItem {
        tool_name: Some("search".to_owned()),
        origin: None,
        created_by: None,
        key_hash: Some("key-hash".to_owned()),
        team_id: Some("team-1".to_owned()),
        key_alias: Some("alias".to_owned()),
        user_agent: Some("agent/1".to_owned()),
    };
    batch_upsert_tools(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &[discovered],
        now,
    )
    .await;
    let tool = stored_tool(&db, "search").await;
    assert_eq!(tool.tool_id, "fixed-id");
    assert_eq!(tool.origin.as_deref(), Some("user_defined"));
    assert_eq!(tool.created_by.as_deref(), Some("system"));
    assert_eq!(tool.updated_by.as_deref(), Some("system"));
    assert_eq!(tool.input_policy, ToolInputPolicy::Untrusted);
    assert_eq!(tool.output_policy, ToolOutputPolicy::Untrusted);
    assert_eq!(tool.call_count, 1);
    assert_eq!(tool.key_hash.as_deref(), Some("key-hash"));
    assert_eq!(tool.team_id.as_deref(), Some("team-1"));
    assert_eq!(tool.key_alias.as_deref(), Some("alias"));
    assert_eq!(tool.user_agent.as_deref(), Some("agent/1"));
    assert_eq!(tool.last_used_at, Some(Timestamp::Naive(now.naive_utc())));
    assert!(tool.created_at.is_some());
    db.close().await;
}

#[tokio::test]
async fn discovery_keeps_the_first_identity_and_increments_the_count() {
    let db = Db::start().await;
    let ids = &UuidToolIdGenerator;
    let first = identified_item("search", "system", "seed", "k1", None);
    batch_upsert_tools(
        Some(db.client().await),
        ids,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &[first],
        instant(),
    )
    .await;
    let second = identified_item("search", "user_defined", "other", "k2", Some("team-2"));
    batch_upsert_tools(
        Some(db.client().await),
        ids,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &[second],
        later_instant(),
    )
    .await;
    let tool = stored_tool(&db, "search").await;
    let tool_id = tool.tool_id.as_str();
    assert!(
        uuid::Uuid::parse_str(tool_id).is_ok(),
        "the generated tool id is a uuid, got {tool_id}"
    );
    assert_eq!(tool.origin.as_deref(), Some("system"));
    assert_eq!(tool.created_by.as_deref(), Some("seed"));
    assert_eq!(tool.updated_by.as_deref(), Some("seed"));
    assert_eq!(tool.key_hash.as_deref(), Some("k1"));
    assert_eq!(tool.team_id, None);
    assert_eq!(tool.call_count, 2);
    let used_at = Timestamp::Naive(later_instant().naive_utc());
    assert_eq!(tool.last_used_at.as_ref(), Some(&used_at));
    assert_eq!(tool.updated_at.as_ref(), Some(&used_at));
    db.close().await;
}

#[tokio::test]
async fn discovery_preserves_an_administrative_policy() {
    let db = Db::start().await;
    store_trusted(&db, "search", instant()).await;
    batch_upsert_tools(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &[item(Some("search"))],
        later_instant(),
    )
    .await;
    let tool = stored_tool(&db, "search").await;
    assert_eq!(tool.input_policy, ToolInputPolicy::Trusted);
    assert_eq!(tool.output_policy, ToolOutputPolicy::Trusted);
    assert_eq!(tool.call_count, 1);
    db.close().await;
}

#[tokio::test]
async fn discovery_ignores_items_without_a_tool_name() {
    let db = Db::start().await;
    let (logs, guard) = captured_logs();
    batch_upsert_tools(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &[item(None), item(Some("")), item(Some("kept"))],
        instant(),
    )
    .await;
    drop(guard);
    assert!(
        logs.text()
            .contains("tool_registry_writer: upserted 1 tool(s)")
    );
    let client = db.client().await;
    assert!(get_tool(Some(&client), "").await.is_none());
    let count: i64 = client
        .query_one("SELECT count(*) FROM \"LiteLLM_ToolTable\"", &[])
        .await
        .expect("the tool count")
        .get(0);
    assert_eq!(count, 1);
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn discovery_stops_at_the_first_failed_item_without_rolling_back() {
    let db = Db::start().await;
    db.batch(POISON_TOOL_TRIGGER).await;
    let (logs, guard) = captured_logs();
    let items = [
        item(Some("first")),
        item(Some("poison")),
        item(Some("after")),
    ];
    batch_upsert_tools(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &items,
        instant(),
    )
    .await;
    drop(guard);
    let client = db.client().await;
    assert!(get_tool(Some(&client), "first").await.is_some());
    assert!(get_tool(Some(&client), "poison").await.is_none());
    assert!(get_tool(Some(&client), "after").await.is_none());
    drop(client);
    let text = logs.text();
    assert!(text.contains("tool_registry_writer batch_upsert_tools error"));
    assert!(text.contains("poison tool"));
    db.close().await;
}

#[tokio::test]
async fn discovery_stops_at_the_deadline() {
    let db = Db::start().await;
    let holder = db.client().await;
    holder
        .batch_execute(TABLE_LOCK)
        .await
        .expect("the table lock");
    let (logs, guard) = captured_logs();
    batch_upsert_tools(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: Duration::from_millis(100),
            canceller: &TlsCanceller { tls: NoTls },
        },
        &[item(Some("search"))],
        instant(),
    )
    .await;
    drop(guard);
    assert!(
        logs.text()
            .contains("the statement did not finish within 100ms")
    );
    assert_eq!(wait_until_no_lock_waiters(&db).await, 0);
    holder
        .batch_execute("ROLLBACK")
        .await
        .expect("the rollback");
    drop(holder);
    let client = db.client().await;
    assert!(get_tool(Some(&client), "search").await.is_none());
    drop(client);
    db.close().await;
}

#[tokio::test]
async fn policy_update_creates_an_undiscovered_tool() {
    let db = Db::start().await;
    let now = instant();
    let update = ToolPolicyUpdate {
        tool_name: "undeclared",
        updated_by: Some("admin"),
        input_policy: Some(ToolInputPolicy::Trusted),
        output_policy: Some(ToolOutputPolicy::Trusted),
    };
    let returned = update_tool_policy(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &update,
        now,
    )
    .await
    .expect("the stored tool");
    assert_eq!(returned.tool_id, "fixed-id");
    assert_eq!(returned.tool_name, "undeclared");
    assert_eq!(returned.input_policy, ToolInputPolicy::Trusted);
    assert_eq!(returned.output_policy, ToolOutputPolicy::Trusted);
    assert_eq!(returned.call_count, 0);
    assert_eq!(returned.origin, None);
    assert_eq!(returned.created_by.as_deref(), Some("admin"));
    assert_eq!(returned.updated_by.as_deref(), Some("admin"));
    assert_eq!(returned.created_at, Some(Timestamp::Naive(now.naive_utc())));
    assert_eq!(returned.updated_at, Some(Timestamp::Naive(now.naive_utc())));
    db.close().await;
}

#[tokio::test]
async fn policy_update_defaults_absent_policies_and_the_actor() {
    let db = Db::start().await;
    let update = ToolPolicyUpdate {
        tool_name: "defaulted",
        updated_by: None,
        input_policy: None,
        output_policy: None,
    };
    let returned = update_tool_policy(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &update,
        instant(),
    )
    .await
    .expect("the stored tool");
    assert_eq!(returned.input_policy, ToolInputPolicy::Untrusted);
    assert_eq!(returned.output_policy, ToolOutputPolicy::Untrusted);
    assert_eq!(returned.created_by.as_deref(), Some("system"));
    assert_eq!(returned.updated_by.as_deref(), Some("system"));
    db.close().await;
}

#[tokio::test]
async fn policy_update_preserves_unspecified_policies() {
    let db = Db::start().await;
    store_trusted(&db, "search", instant()).await;
    batch_upsert_tools(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &[item(Some("search"))],
        instant(),
    )
    .await;
    let changed = ToolPolicyUpdate {
        tool_name: "search",
        updated_by: Some("second"),
        input_policy: None,
        output_policy: Some(ToolOutputPolicy::Untrusted),
    };
    let returned = update_tool_policy(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &changed,
        later_instant(),
    )
    .await
    .expect("the updated tool");
    assert_eq!(returned.input_policy, ToolInputPolicy::Trusted);
    assert_eq!(returned.output_policy, ToolOutputPolicy::Untrusted);
    assert_eq!(returned.created_by.as_deref(), Some("system"));
    assert_eq!(returned.updated_by.as_deref(), Some("second"));
    assert_eq!(returned.call_count, 1);
    assert_eq!(
        returned.updated_at,
        Some(Timestamp::Naive(later_instant().naive_utc()))
    );
    db.close().await;
}

#[tokio::test]
async fn policy_update_failure_returns_none_and_logs() {
    let db = Db::start().await;
    db.batch(POISON_TOOL_TRIGGER).await;
    let (logs, guard) = captured_logs();
    let update = ToolPolicyUpdate {
        tool_name: "poison",
        updated_by: None,
        input_policy: Some(ToolInputPolicy::Trusted),
        output_policy: None,
    };
    let returned = update_tool_policy(
        Some(db.client().await),
        &FixedIds,
        StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
        &update,
        instant(),
    )
    .await;
    drop(guard);
    assert!(returned.is_none());
    let client = db.client().await;
    assert!(get_tool(Some(&client), "poison").await.is_none());
    drop(client);
    assert!(
        logs.text()
            .contains("tool_registry_writer update_tool_policy error")
    );
    db.close().await;
}

#[tokio::test]
async fn blocking_lists_a_tool_once_and_keeps_the_existing_entries() {
    let db = Db::start().await;
    db.batch(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (\"object_permission_id\", \"blocked_tools\") \
         VALUES ('perm-1', ARRAY['existing'])",
    )
    .await;
    check_blocked_change(
        db.client().await,
        "perm-1",
        "search",
        BlockedChange::Add,
        true,
    )
    .await;
    check_blocked_change(
        db.client().await,
        "perm-1",
        "search",
        BlockedChange::Add,
        true,
    )
    .await;
    assert_eq!(
        blocked_list(&db, "perm-1").await,
        Some(vec!["existing".to_owned(), "search".to_owned()])
    );
    db.close().await;
}

#[tokio::test]
async fn blocking_an_unknown_permission_returns_false() {
    let db = Db::start().await;
    check_blocked_change(
        db.client().await,
        "missing",
        "search",
        BlockedChange::Add,
        false,
    )
    .await;
    check_blocked_change(
        db.client().await,
        "missing",
        "search",
        BlockedChange::Remove,
        false,
    )
    .await;
    db.close().await;
}

#[tokio::test]
async fn blocking_a_null_list_starts_an_empty_list() {
    let db = Db::start().await;
    db.batch(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (\"object_permission_id\", \"blocked_tools\") \
         VALUES ('perm-null', NULL)",
    )
    .await;
    check_blocked_change(
        db.client().await,
        "perm-null",
        "search",
        BlockedChange::Add,
        true,
    )
    .await;
    assert_eq!(
        blocked_list(&db, "perm-null").await,
        Some(vec!["search".to_owned()])
    );
    db.close().await;
}

#[tokio::test]
async fn unblocking_removes_the_tool_and_reports_a_missing_one() {
    let db = Db::start().await;
    db.batch(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (\"object_permission_id\", \"blocked_tools\") \
         VALUES ('perm-remove', ARRAY['keep', 'drop'])",
    )
    .await;
    check_blocked_change(
        db.client().await,
        "perm-remove",
        "drop",
        BlockedChange::Remove,
        true,
    )
    .await;
    check_blocked_change(
        db.client().await,
        "perm-remove",
        "drop",
        BlockedChange::Remove,
        false,
    )
    .await;
    check_blocked_change(
        db.client().await,
        "perm-remove",
        "absent",
        BlockedChange::Remove,
        false,
    )
    .await;
    assert_eq!(
        blocked_list(&db, "perm-remove").await,
        Some(vec!["keep".to_owned()])
    );
    db.close().await;
}

#[tokio::test]
async fn blocked_list_writes_report_a_failed_update() {
    let db = Db::start().await;
    db.batch(LOCKED_PERMISSION_TRIGGER).await;
    db.batch(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (\"object_permission_id\", \"blocked_tools\") \
         VALUES ('locked-add', ARRAY['present']), ('locked-remove', ARRAY['present'])",
    )
    .await;
    let (logs, guard) = captured_logs();
    check_blocked_change(
        db.client().await,
        "locked-add",
        "search",
        BlockedChange::Add,
        false,
    )
    .await;
    check_blocked_change(
        db.client().await,
        "locked-remove",
        "present",
        BlockedChange::Remove,
        false,
    )
    .await;
    drop(guard);
    let text = logs.text();
    assert!(text.contains("add_tool_to_object_permission_blocked error"));
    assert!(text.contains("remove_tool_from_object_permission_blocked error"));
    assert_eq!(
        blocked_list(&db, "locked-add").await,
        Some(vec!["present".to_owned()])
    );
    assert_eq!(
        blocked_list(&db, "locked-remove").await,
        Some(vec!["present".to_owned()])
    );
    db.close().await;
}

#[tokio::test]
async fn blocked_list_read_failures_report_false_and_log() {
    let db = Db::start().await;
    db.batch(
        "ALTER TABLE \"LiteLLM_ObjectPermissionTable\" RENAME TO \"LiteLLM_ObjectPermissionTable_moved\"",
    )
    .await;
    let (logs, guard) = captured_logs();
    check_blocked_change(
        db.client().await,
        "perm-1",
        "search",
        BlockedChange::Add,
        false,
    )
    .await;
    check_blocked_change(
        db.client().await,
        "perm-1",
        "search",
        BlockedChange::Remove,
        false,
    )
    .await;
    drop(guard);
    let text = logs.text();
    assert!(text.contains("add_tool_to_object_permission_blocked error"));
    assert!(text.contains("remove_tool_from_object_permission_blocked error"));
    db.close().await;
}

#[tokio::test]
async fn refresh_loads_tool_maps_and_permissions() {
    let db = Db::start().await;
    store_trusted(&db, "search", instant()).await;
    db.batch(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (\"object_permission_id\", \"blocked_tools\") \
         VALUES ('perm-1', ARRAY['search'])",
    )
    .await;
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let (logs, guard) = captured_logs();
    sync(&db, &registry).await.expect("a loaded registry");
    drop(guard);
    let state = registry.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(state.initialized);
    assert_eq!(
        state.input_policies.get("search"),
        Some(&ToolInputPolicy::Trusted)
    );
    assert_eq!(
        state.output_policies.get("search"),
        Some(&ToolOutputPolicy::Trusted)
    );
    assert!(
        logs.text().contains(
            "ToolPolicyRegistry: synced 1 tool policies and 1 object permissions from DB"
        )
    );
    drop(state);
    db.close().await;
}

#[tokio::test]
async fn refresh_replaces_the_blocked_list() {
    let db = Db::start().await;
    db.batch(
        "INSERT INTO \"LiteLLM_ObjectPermissionTable\" (\"object_permission_id\", \"blocked_tools\") \
         VALUES ('perm-1', ARRAY['search'])",
    )
    .await;
    let registry = Mutex::new(ToolPolicyRegistry::default());
    sync(&db, &registry).await.expect("a loaded registry");
    db.batch(
        "UPDATE \"LiteLLM_ObjectPermissionTable\" SET \"blocked_tools\" = ARRAY['other'] \
         WHERE \"object_permission_id\" = 'perm-1'",
    )
    .await;
    sync(&db, &registry).await.expect("a reloaded registry");
    let state = registry.lock().unwrap_or_else(PoisonError::into_inner);
    let released = state.effective_policies(&["search".to_owned()], Some("perm-1"), None);
    assert_eq!(released.get("search"), Some(&ToolInputPolicy::Untrusted));
    let blocked = state.effective_policies(&["other".to_owned()], Some("perm-1"), None);
    assert_eq!(blocked.get("other"), Some(&ToolInputPolicy::Blocked));
    drop(state);
    db.close().await;
}

#[tokio::test]
async fn a_failed_permission_read_keeps_the_new_tool_maps_without_initializing() {
    let db = Db::start().await;
    store_trusted(&db, "search", instant()).await;
    db.batch(
        "ALTER TABLE \"LiteLLM_ObjectPermissionTable\" RENAME TO \"LiteLLM_ObjectPermissionTable_moved\"",
    )
    .await;
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let (logs, guard) = captured_logs();
    let result = sync(&db, &registry).await;
    drop(guard);
    assert!(result.is_err());
    let state = registry.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(!state.initialized);
    assert_eq!(
        state.input_policies.get("search"),
        Some(&ToolInputPolicy::Trusted)
    );
    assert!(state.blocked_tools_by_permission.is_empty());
    drop(state);
    assert!(
        logs.text()
            .contains("ToolPolicyRegistry sync_tool_policy_from_db error")
    );
    db.close().await;
}

#[tokio::test]
async fn refresh_does_not_reconnect_for_a_missing_table() {
    let db = Db::start().await;
    db.batch("ALTER TABLE \"LiteLLM_ToolTable\" RENAME TO \"LiteLLM_ToolTable_moved\"")
        .await;
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let store = ToolPolicyStore {
        database: &db.store,
        registry: &registry,
        bounds: StatementBounds {
            deadline: DEADLINE,
            canceller: &TlsCanceller { tls: NoTls },
        },
    };
    let mut reconnect = Reconnect::default();
    let result = sync_tool_policy_from_db(&store, Some(&mut reconnect)).await;
    assert!(reconnect.reasons().is_empty());
    match result.expect_err("a missing table fails the read") {
        ToolPolicyStoreError::Read {
            error_type, source, ..
        } => {
            assert_eq!(error_type, ErrorType::Other);
            assert!(
                matches!(source, NativeDbError::SqlState { code, .. } if code == SqlState::UNDEFINED_TABLE)
            );
        }
        other => panic!("expected a typed read error, got {other:?}"),
    }
    db.close().await;
}

#[tokio::test]
async fn refresh_cancels_a_read_past_the_deadline() {
    let db = Db::start().await;
    let holder = db.client().await;
    holder
        .batch_execute(TABLE_LOCK)
        .await
        .expect("the table lock");
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let store = ToolPolicyStore {
        database: &db.store,
        registry: &registry,
        bounds: StatementBounds {
            deadline: Duration::from_millis(100),
            canceller: &TlsCanceller { tls: NoTls },
        },
    };
    let mut reconnect = Reconnect::default();
    let result = sync_tool_policy_from_db(&store, Some(&mut reconnect)).await;
    assert!(matches!(result, Err(ToolPolicyStoreError::Deadline { .. })));
    assert!(reconnect.reasons().is_empty());
    assert!(
        !registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .initialized
    );
    assert_eq!(wait_until_no_lock_waiters(&db).await, 0);
    holder
        .batch_execute("ROLLBACK")
        .await
        .expect("the rollback");
    drop(holder);
    db.close().await;
}

#[tokio::test]
async fn refresh_fails_on_an_unreadable_tool_row() {
    let db = Db::start().await;
    db.batch(
        "INSERT INTO \"LiteLLM_ToolTable\" (\"tool_id\", \"tool_name\", \"input_policy\") \
         VALUES ('t1', 'bogus', 'bogus')",
    )
    .await;
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let result = sync(&db, &registry).await;
    assert!(result.is_err());
    assert!(
        !registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .initialized
    );
    db.close().await;
}

#[tokio::test]
async fn refresh_fails_on_an_unreadable_permission_row() {
    let db = Db::start().await;
    db.batch(
        "ALTER TABLE \"LiteLLM_ObjectPermissionTable\" ALTER COLUMN \"blocked_tools\" DROP DEFAULT; \
         ALTER TABLE \"LiteLLM_ObjectPermissionTable\" ALTER COLUMN \"blocked_tools\" TYPE text \
         USING array_to_string(\"blocked_tools\", ','); \
         INSERT INTO \"LiteLLM_ObjectPermissionTable\" (\"object_permission_id\", \"blocked_tools\") \
         VALUES ('perm-1', 'search')",
    )
    .await;
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let result = sync(&db, &registry).await;
    assert!(result.is_err());
    assert!(
        !registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .initialized
    );
    db.close().await;
}

#[tokio::test]
async fn a_tls_required_cancel_stops_the_read_over_tls() {
    let db = Db::start().await;
    let server_url = db.fixture.options.url.expose_secret();
    let front = TlsFront::start(server_url).await;
    let store = front.store(server_url);
    let holder = db.client().await;
    holder
        .batch_execute(TABLE_LOCK)
        .await
        .expect("the table lock");
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let canceller = front.trusted_connector();
    let policy_store = ToolPolicyStore {
        database: &store,
        registry: &registry,
        bounds: StatementBounds {
            deadline: Duration::from_millis(500),
            canceller: &TlsCanceller { tls: canceller },
        },
    };
    let (logs, guard) = captured_logs();
    let result = sync_tool_policy_from_db(&policy_store, None::<&mut Reconnect>).await;
    drop(guard);
    assert!(matches!(result, Err(ToolPolicyStoreError::Deadline { .. })));
    assert_eq!(
        front.handshakes(),
        2,
        "the session and its cancel request both spoke TLS"
    );
    assert!(!logs.text().contains("could not be cancelled"));
    assert_eq!(wait_until_no_lock_waiters(&db).await, 0);
    holder
        .batch_execute("ROLLBACK")
        .await
        .expect("the rollback");
    drop(holder);
    sync_tool_policy_from_db(&policy_store, None::<&mut Reconnect>)
        .await
        .expect("the read after the cancel");
    assert_eq!(
        front.handshakes(),
        2,
        "the cancelled session stayed in the pool"
    );
    store.pool.expect("the TLS pool").close();
    yield_now().await;
    drop(front);
    yield_now().await;
    db.close().await;
}

#[tokio::test]
async fn an_undeliverable_cancel_discards_the_connection() {
    let db = Db::start().await;
    let server_url = db.fixture.options.url.expose_secret();
    let front = TlsFront::start(server_url).await;
    let store = front.store(server_url);
    let holder = db.client().await;
    holder
        .batch_execute(TABLE_LOCK)
        .await
        .expect("the table lock");
    let registry = Mutex::new(ToolPolicyRegistry::default());
    let canceller = TlsFront::untrusted_connector();
    let policy_store = ToolPolicyStore {
        database: &store,
        registry: &registry,
        bounds: StatementBounds {
            deadline: Duration::from_millis(500),
            canceller: &TlsCanceller { tls: canceller },
        },
    };
    let (logs, guard) = captured_logs();
    let result = sync_tool_policy_from_db(&policy_store, None::<&mut Reconnect>).await;
    drop(guard);
    assert!(matches!(result, Err(ToolPolicyStoreError::Deadline { .. })));
    assert!(logs.text().contains("could not be cancelled"));
    let pool = store.pool.as_ref().expect("the TLS pool");
    assert_eq!(
        pool.status().size,
        0,
        "the timed out connection left the pool"
    );
    assert_eq!(pool.status().available, 0);
    holder
        .batch_execute("ROLLBACK")
        .await
        .expect("the rollback");
    drop(holder);
    store.pool.expect("the TLS pool").close();
    yield_now().await;
    drop(front);
    yield_now().await;
    db.close().await;
}

#![cfg(test)]

mod postgres;
mod support;

use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_auth::auth_cache::WriteOptions;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::DualCache;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use deadpool_postgres::GenericClient;
use deadpool_postgres::Transaction;
use tokio_postgres::types::ToSql;

use self::postgres::{
    Database, RejectingTx, age_row, agent_groups, insert_agent, insert_key, insert_team,
    key_groups, seeded_at, team_exists, team_groups, updated_at,
};
use self::support::{
    RegistryOp, ScriptedCaches, ScriptedRegistry, agent, gateway, key_auth, key_view, team_object,
};
use crate::access_group_member_effects::agents::{
    detach_group_from_agents, detach_group_from_registry, without_access_group,
};
use crate::access_group_member_effects::assignment::{
    MembershipChange, MembershipTarget, change_membership,
};
use crate::access_group_member_effects::cache::{
    CacheTarget, GatewayMemberCaches, change_cached_membership,
};
use crate::access_group_member_effects::error::{MemberEffectsError, RegistrationRejected};
use crate::access_group_read::protocols::postgres::PgAccessGroupTables;

fn groups(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| String::from(*id)).collect()
}

async fn change_rows<C>(
    tables: &PgAccessGroupTables<'_, C>,
    target: MembershipTarget,
    ids: &[&str],
    access_group_id: &str,
    change: MembershipChange,
) where
    C: GenericClient + Sync,
{
    let listed = groups(ids);
    change_membership(tables, target, &listed, access_group_id, change)
        .await
        .expect("the membership change runs");
}

#[tokio::test]
async fn assignment_missing_rows() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    insert_team(&database, "t1", Some(&groups(&["g2"]))).await;
    insert_key(&database, "tok-1", Some(&groups(&["g2"]))).await;
    change_rows(
        &tables,
        MembershipTarget::Team,
        &["t1", "t-missing"],
        "g1",
        MembershipChange::Add,
    )
    .await;
    change_rows(
        &tables,
        MembershipTarget::Key,
        &["tok-1", "tok-missing"],
        "g1",
        MembershipChange::Add,
    )
    .await;
    assert_missing_rows_skipped(&database).await;
    change_rows(
        &tables,
        MembershipTarget::Team,
        &["t-missing"],
        "g1",
        MembershipChange::Remove,
    )
    .await;
    database.close().await;
}

async fn assert_missing_rows_skipped(database: &Database) {
    assert_eq!(
        team_groups(database, "t1").await,
        Some(groups(&["g2", "g1"]))
    );
    assert_eq!(
        key_groups(database, "tok-1").await,
        Some(groups(&["g2", "g1"]))
    );
    assert!(!team_exists(database, "t-missing").await);
}

async fn seed_assignment_rows(database: &Database) {
    insert_team(database, "t-null", None).await;
    insert_team(database, "t-null-remove", None).await;
    insert_team(database, "t-present", Some(&groups(&["g1"]))).await;
    insert_team(database, "t-append", Some(&groups(&["g2", "g2"]))).await;
    insert_team(database, "t-order", Some(&groups(&["g3", "g1", "g2"]))).await;
    insert_key(database, "tok-null", None).await;
    insert_key(database, "tok-present", Some(&groups(&["g2"]))).await;
    insert_key(database, "tok-dup", Some(&groups(&["g1", "g1", "g2"]))).await;
    insert_key(database, "tok-absent", Some(&groups(&["g2"]))).await;
    age_row(database, "LiteLLM_TeamTable", "team_id", "t-present").await;
    age_row(database, "LiteLLM_VerificationToken", "token", "tok-absent").await;
}

#[derive(Clone, Copy)]
enum AssignedTable {
    Team,
    Key,
}

async fn assigned_groups(
    database: &Database,
    table: AssignedTable,
    id: &str,
) -> Option<Vec<String>> {
    match table {
        AssignedTable::Team => team_groups(database, id).await,
        AssignedTable::Key => key_groups(database, id).await,
    }
}

async fn assert_groups(database: &Database, table: AssignedTable, id: &str, expected: &[&str]) {
    assert_eq!(
        assigned_groups(database, table, id).await,
        Some(groups(expected))
    );
}

async fn assert_assignment_rows(database: &Database) {
    assert_groups(database, AssignedTable::Team, "t-null", &["g1"]).await;
    assert_groups(database, AssignedTable::Team, "t-present", &["g1"]).await;
    assert_groups(
        database,
        AssignedTable::Team,
        "t-append",
        &["g2", "g2", "g1"],
    )
    .await;
    assert_groups(database, AssignedTable::Team, "t-order", &["g3", "g2"]).await;
    assert_eq!(team_groups(database, "t-null-remove").await, None);
    assert_eq!(
        updated_at(database, "LiteLLM_TeamTable", "team_id", "t-present").await,
        seeded_at()
    );
    assert_groups(database, AssignedTable::Key, "tok-null", &["g1"]).await;
    assert_groups(database, AssignedTable::Key, "tok-present", &["g2", "g1"]).await;
    assert_groups(database, AssignedTable::Key, "tok-dup", &["g2"]).await;
    assert_groups(database, AssignedTable::Key, "tok-absent", &["g2"]).await;
    assert_eq!(
        updated_at(database, "LiteLLM_VerificationToken", "token", "tok-absent").await,
        seeded_at()
    );
}

#[tokio::test]
async fn assignment_null_duplicate_and_order() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    seed_assignment_rows(&database).await;
    change_rows(
        &tables,
        MembershipTarget::Team,
        &["t-null", "t-present", "t-append"],
        "g1",
        MembershipChange::Add,
    )
    .await;
    change_rows(
        &tables,
        MembershipTarget::Team,
        &["t-order", "t-null-remove"],
        "g1",
        MembershipChange::Remove,
    )
    .await;
    change_rows(
        &tables,
        MembershipTarget::Key,
        &["tok-null", "tok-present"],
        "g1",
        MembershipChange::Add,
    )
    .await;
    change_rows(
        &tables,
        MembershipTarget::Key,
        &["tok-dup", "tok-absent"],
        "g1",
        MembershipChange::Remove,
    )
    .await;
    assert_assignment_rows(&database).await;
    database.close().await;
}

async fn reject_second_key(transaction: &Transaction<'_>) -> MemberEffectsError {
    let tables = PgAccessGroupTables {
        client: transaction,
    };
    let rejecting = RejectingTx {
        inner: tables,
        reject: "tok-two",
    };
    let tokens = groups(&["tok-one", "tok-two"]);
    change_membership(
        &rejecting,
        MembershipTarget::Key,
        &tokens,
        "g1",
        MembershipChange::Remove,
    )
    .await
    .expect_err("the second write fails")
}

async fn assert_first_key_updated(transaction: &Transaction<'_>) {
    let parameters: [&(dyn ToSql + Sync); 1] = [&"tok-one"];
    let statement =
        r#"SELECT "access_group_ids" FROM "LiteLLM_VerificationToken" WHERE "token" = $1"#;
    let row = transaction
        .query_one(statement, &parameters)
        .await
        .expect("the first key is readable in the transaction");
    let updated: Option<Vec<String>> = row.get(0);
    assert_eq!(updated, Some(Vec::new()));
}

async fn assert_rolled_back(database: &Database) {
    assert_eq!(key_groups(database, "tok-one").await, Some(groups(&["g1"])));
    assert_eq!(key_groups(database, "tok-two").await, Some(groups(&["g1"])));
}

#[tokio::test]
async fn assignment_transaction_failure() {
    let database = Database::start().await;
    insert_key(&database, "tok-one", Some(&groups(&["g1"]))).await;
    insert_key(&database, "tok-two", Some(&groups(&["g1"]))).await;
    let mut client = database.client().await;
    let transaction = client.transaction().await.expect("the transaction opens");
    let error = reject_second_key(&transaction).await;
    assert!(matches!(error, MemberEffectsError::Database(_)));
    assert_first_key_updated(&transaction).await;
    transaction
        .rollback()
        .await
        .expect("the transaction rolls back");
    assert_rolled_back(&database).await;
    database.close().await;
}

#[tokio::test]
async fn agent_db_detach() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    insert_agent(&database, "agent-one", Some(&groups(&["g1", "g2"]))).await;
    insert_agent(&database, "agent-dup", Some(&groups(&["g1", "g1", "g2"]))).await;
    insert_agent(&database, "agent-other", Some(&groups(&["g3"]))).await;
    insert_agent(&database, "agent-null", None).await;
    let detached = detach_group_from_agents(&tables, "g1")
        .await
        .expect("the agent detach runs");
    let mut sorted = detached;
    sorted.sort();
    assert_eq!(sorted, groups(&["agent-dup", "agent-one"]));
    assert_eq!(
        agent_groups(&database, "agent-one").await,
        Some(groups(&["g2"]))
    );
    assert_eq!(
        agent_groups(&database, "agent-dup").await,
        Some(groups(&["g2"]))
    );
    assert_eq!(
        agent_groups(&database, "agent-other").await,
        Some(groups(&["g3"]))
    );
    assert_eq!(agent_groups(&database, "agent-null").await, None);
    database.close().await;
}

#[test]
fn filter_preserves_unrelated_order() {
    assert!(without_access_group(None, "g1").is_empty());
    let carried = groups(&["b", "g1", "a", "g1", "c", "b"]);
    let filtered = without_access_group(Some(&carried), "g1");
    assert_eq!(filtered, groups(&["b", "a", "c", "b"]));
    let only = groups(&["g1"]);
    assert!(without_access_group(Some(&only), "g1").is_empty());
}

#[test]
fn registry_snapshot_and_missing() {
    let registry = ScriptedRegistry::default();
    registry.seed(agent("agent-one", "one", Some(groups(&["g1", "g2"]))));
    registry.seed(agent("agent-two", "two", Some(groups(&["g3"]))));
    let ids = groups(&["agent-one", "missing", "agent-two"]);
    detach_group_from_registry(&registry, &ids, "g1").expect("the registry detach runs");
    let expected = vec![
        RegistryOp::Find("agent-one".to_owned()),
        RegistryOp::Find("missing".to_owned()),
        RegistryOp::Find("agent-two".to_owned()),
        RegistryOp::Deregister("one".to_owned()),
        RegistryOp::Register {
            agent_id: "agent-one".to_owned(),
            agent_name: "one".to_owned(),
            groups: Some(groups(&["g2"])),
        },
        RegistryOp::Deregister("two".to_owned()),
        RegistryOp::Register {
            agent_id: "agent-two".to_owned(),
            agent_name: "two".to_owned(),
            groups: Some(groups(&["g3"])),
        },
    ];
    assert_eq!(registry.ops(), expected);
    let reregistered = registry
        .agent("one")
        .expect("the first agent registers again");
    assert_eq!(reregistered.access_group_ids, Some(groups(&["g2"])));
}

#[test]
fn registry_register_failure() {
    let registry = ScriptedRegistry::default();
    registry.seed(agent("agent-one", "one", Some(groups(&["g1", "g2"]))));
    registry.seed(agent("agent-two", "two", Some(groups(&["g1"]))));
    registry.seed(agent("agent-three", "three", Some(groups(&["g1"]))));
    registry.rejecting("two");
    let ids = groups(&["agent-one", "agent-two", "agent-three"]);
    let error = detach_group_from_registry(&registry, &ids, "g1")
        .expect_err("the second register is rejected");
    assert!(matches!(error, RegistrationRejected));
    let reregistered = registry
        .agent("one")
        .expect("the first agent registers again");
    assert_eq!(reregistered.access_group_ids, Some(groups(&["g2"])));
    assert!(registry.agent("two").is_none());
    let ops = registry.ops();
    assert!(ops.contains(&RegistryOp::Deregister("two".to_owned())));
    assert!(!ops.contains(&RegistryOp::Deregister("three".to_owned())));
    assert!(!ops.contains(&RegistryOp::Register {
        agent_id: "agent-three".to_owned(),
        agent_name: "three".to_owned(),
        groups: Some(Vec::new()),
    }));
}

#[test]
fn agent_registry_effects_delegate() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("agent-one", "one", Some(groups(&["g1", "g2"]))));
    let ids = groups(&["agent-one"]);
    detach_group_from_registry(&registry, &ids, "g1")
        .expect("the real registry accepts the re-registration");
    let stored = registry
        .get_agent_by_id("agent-one")
        .expect("the agent stays registered");
    assert_eq!(stored.access_group_ids, Some(groups(&["g2"])));
    assert_eq!(stored.agent_name, "one");
}

fn seed_cached(
    caches: &ScriptedCaches,
    teams: &[(&str, Option<&[&str]>)],
    keys: &[(&str, Option<&[&str]>)],
) {
    for &(team_id, listed) in teams {
        caches.seed_team(team_id, team_object(team_id, listed.map(groups)));
    }
    for &(token, listed) in keys {
        caches.seed_key(token, key_auth(token, listed.map(groups)));
    }
}

#[tokio::test]
async fn cache_miss_no_refill() {
    let caches = ScriptedCaches::default();
    let team_ids = groups(&["t1", "t2"]);
    change_cached_membership(
        &caches,
        CacheTarget::Team,
        &team_ids,
        "g1",
        MembershipChange::Add,
    )
    .await
    .expect("a missing team is skipped");
    assert_eq!(caches.reads(), groups(&["team_id:t1", "team_id:t2"]));
    assert!(caches.written().is_empty());
    let tokens = groups(&["tok-1", "tok-2"]);
    change_cached_membership(
        &caches,
        CacheTarget::Key,
        &tokens,
        "g1",
        MembershipChange::Remove,
    )
    .await
    .expect("a missing key is skipped");
    assert_eq!(
        caches.reads(),
        groups(&["team_id:t1", "team_id:t2", "tok-1", "tok-2"])
    );
    assert!(caches.written().is_empty());
}

fn assert_cache_adds(caches: &ScriptedCaches) {
    assert_eq!(
        caches.written(),
        groups(&["t-null", "t-other", "tok-null", "tok-other"])
    );
    let null_team = caches
        .stored_team("t-null")
        .expect("the null team is patched");
    assert_eq!(
        null_team.team.fields.base.access_group_ids,
        Some(groups(&["g1"]))
    );
    assert_eq!(
        null_team.team.fields.base.team_alias.as_deref(),
        Some("alias-t-null")
    );
    assert_eq!(null_team.team.fields.base.tpm_limit, Some(7_i64));
    let other_team = caches
        .stored_team("t-other")
        .expect("the other team is patched");
    assert_eq!(
        other_team.team.fields.base.access_group_ids,
        Some(groups(&["g2", "g2", "g1"]))
    );
    assert_added_keys(caches);
}

fn assert_added_keys(caches: &ScriptedCaches) {
    let null_key = key_view(
        &caches
            .stored_key("tok-null")
            .expect("the null key is patched"),
    );
    assert_eq!(null_key.access_group_ids, Some(groups(&["g1"])));
    assert_eq!(null_key.user_id.as_deref(), Some("user-1"));
    assert_eq!(null_key.key_alias.as_deref(), Some("key-one"));
    let other_key = key_view(
        &caches
            .stored_key("tok-other")
            .expect("the other key is patched"),
    );
    assert_eq!(
        other_key.access_group_ids,
        Some(groups(&["g2", "g2", "g1"]))
    );
}

#[tokio::test]
async fn cache_add_null_existing_and_duplicate() {
    let caches = ScriptedCaches::default();
    seed_cached(
        &caches,
        &[
            ("t-null", None),
            ("t-present", Some(&["g1"])),
            ("t-other", Some(&["g2", "g2"])),
        ],
        &[
            ("tok-null", None),
            ("tok-present", Some(&["g1"])),
            ("tok-other", Some(&["g2", "g2"])),
        ],
    );
    let team_ids = groups(&["t-null", "t-present", "t-other"]);
    change_cached_membership(
        &caches,
        CacheTarget::Team,
        &team_ids,
        "g1",
        MembershipChange::Add,
    )
    .await
    .expect("the team add runs");
    let tokens = groups(&["tok-null", "tok-present", "tok-other"]);
    change_cached_membership(
        &caches,
        CacheTarget::Key,
        &tokens,
        "g1",
        MembershipChange::Add,
    )
    .await
    .expect("the key add runs");
    assert_cache_adds(&caches);
}

fn assert_cache_removals(caches: &ScriptedCaches) {
    assert_eq!(caches.written(), groups(&["t-absent", "tok-dup"]));
    let team = caches
        .stored_team("t-absent")
        .expect("the team is rewritten");
    assert_eq!(
        team.team.fields.base.access_group_ids,
        Some(groups(&["g2", "g3"]))
    );
    let key = key_view(&caches.stored_key("tok-dup").expect("the key is rewritten"));
    assert_eq!(key.access_group_ids, Some(groups(&["g2"])));
}

#[tokio::test]
async fn cache_remove_nonempty_absent() {
    let caches = ScriptedCaches::default();
    seed_cached(
        &caches,
        &[
            ("t-absent", Some(&["g2", "g3"])),
            ("t-empty", Some(&[])),
            ("t-null", None),
        ],
        &[
            ("tok-dup", Some(&["g1", "g2", "g1"])),
            ("tok-empty", Some(&[])),
        ],
    );
    let team_ids = groups(&["t-absent", "t-empty", "t-null"]);
    change_cached_membership(
        &caches,
        CacheTarget::Team,
        &team_ids,
        "g1",
        MembershipChange::Remove,
    )
    .await
    .expect("the team remove runs");
    let tokens = groups(&["tok-dup", "tok-empty", "tok-missing"]);
    change_cached_membership(
        &caches,
        CacheTarget::Key,
        &tokens,
        "g1",
        MembershipChange::Remove,
    )
    .await
    .expect("the key remove runs");
    assert_cache_removals(&caches);
}

#[tokio::test]
async fn cache_failure_stops_sequence() {
    let caches = ScriptedCaches::default();
    caches.seed_team("t1", team_object("t1", Some(groups(&["g9"]))));
    caches.seed_team("t2", team_object("t2", Some(groups(&["g9"]))));
    caches.seed_team("t3", team_object("t3", Some(groups(&["g9"]))));
    caches.failing(&["t2"]);
    let team_ids = groups(&["t1", "t2", "t3"]);
    let error = change_cached_membership(
        &caches,
        CacheTarget::Team,
        &team_ids,
        "g1",
        MembershipChange::Add,
    )
    .await
    .expect_err("the second store fails");
    assert!(matches!(error, MemberEffectsError::CacheWrite(_)));
    assert_eq!(caches.reads(), groups(&["team_id:t1", "team_id:t2"]));
    assert_eq!(caches.written(), groups(&["t1"]));
    let stored = caches.stored_team("t1").expect("the first team is patched");
    assert_eq!(
        stored.team.fields.base.access_group_ids,
        Some(groups(&["g9", "g1"]))
    );
}

async fn assert_team_patch(caches: &GatewayMemberCaches<DualCache<LocalMemoryOps>>) {
    let stored = caches
        .cache
        .async_get_typed::<LiteLLMTeamTableCachedObj>("team_id:t1", false)
        .await
        .expect("the team patch reads")
        .expect("the patched team entry");
    assert_eq!(
        stored.team.fields.base.access_group_ids,
        Some(groups(&["g9", "g1"]))
    );
    assert_eq!(
        stored.team.fields.base.team_alias.as_deref(),
        Some("alias-t1")
    );
    assert_eq!(stored.team.fields.base.tpm_limit, Some(7_i64));
    assert!(stored.last_refreshed_at.is_some());
    let missing = caches
        .cache
        .async_get_typed::<LiteLLMTeamTableCachedObj>("team_id:t-missing", false)
        .await
        .expect("the miss reads");
    assert!(missing.is_none());
}

async fn assert_key_patch(caches: &GatewayMemberCaches<DualCache<LocalMemoryOps>>, token: &str) {
    let stored = caches
        .cache
        .async_get_auth(token, false)
        .await
        .expect("the key patch reads")
        .expect("the patched key entry");
    let view = key_view(&stored);
    assert_eq!(view.access_group_ids, Some(groups(&["g1"])));
    assert_eq!(view.user_id.as_deref(), Some("user-1"));
    assert_eq!(view.key_alias.as_deref(), Some("key-one"));
}

async fn seed_gateway(
    caches: &GatewayMemberCaches<DualCache<LocalMemoryOps>>,
    team: &LiteLLMTeamTableCachedObj,
    token: &str,
) {
    caches
        .cache
        .async_set(Some("team_id:t1"), team, WriteOptions::default())
        .await
        .expect("the team seeds");
    let auth = key_auth(token, None);
    caches
        .cache
        .async_set(Some(token), &auth, WriteOptions::default())
        .await
        .expect("the key seeds");
}

#[tokio::test]
async fn gateway_caches_patch_cached_objects() {
    let caches = gateway();
    let team = team_object("t1", Some(groups(&["g9"])));
    let token = "0123456789abcdef".repeat(4);
    seed_gateway(&caches, &team, &token).await;
    let team_ids = groups(&["t1", "t-missing"]);
    Box::pin(change_cached_membership(
        &caches,
        CacheTarget::Team,
        &team_ids,
        "g1",
        MembershipChange::Add,
    ))
    .await
    .expect("the team patch runs");
    let tokens = groups(&[token.as_str()]);
    Box::pin(change_cached_membership(
        &caches,
        CacheTarget::Key,
        &tokens,
        "g1",
        MembershipChange::Add,
    ))
    .await
    .expect("the key patch runs");
    assert_team_patch(&caches).await;
    assert_key_patch(&caches, &token).await;
}

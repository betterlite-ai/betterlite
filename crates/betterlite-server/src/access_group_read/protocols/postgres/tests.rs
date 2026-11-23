#![cfg(test)]

mod support;

use betterlite_db::postgres_store::errors::DbError;
use chrono::NaiveDateTime;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use self::support::Database;
use crate::access_group_read::protocols::error::AccessGroupStoreError;
use crate::access_group_read::protocols::records::{
    AccessGroupChanges, AgentRecord, FieldWrite, NewAccessGroup, TeamRecord,
};
use crate::access_group_read::protocols::tables::{
    AccessGroupTable, AccessGroupTx, AgentTable, KeyTable, TeamTable, TeamWhere,
};

use super::PgAccessGroupTables;

const SEEDED_AT: &str = "2000-01-01 00:00:00";

fn seeded_at() -> NaiveDateTime {
    NaiveDateTime::parse_from_str(SEEDED_AT, "%Y-%m-%d %H:%M:%S")
        .expect("the seed timestamp parses")
}

fn groups(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| String::from(*id)).collect()
}

fn new_group(name: &str, description: Option<&str>, teams: &[&str]) -> NewAccessGroup {
    NewAccessGroup {
        access_group_name: name.to_owned(),
        description: description.map(String::from),
        access_model_names: groups(&["model-a"]),
        access_mcp_server_ids: Vec::new(),
        access_agent_ids: Vec::new(),
        assigned_team_ids: teams.iter().map(|team| String::from(*team)).collect(),
        assigned_key_ids: groups(&["token-a"]),
        created_by: Some("creator".to_owned()),
        updated_by: Some("creator".to_owned()),
    }
}

fn team_ids(teams: &[TeamRecord]) -> Vec<String> {
    let mut ids: Vec<String> = teams.iter().map(|team| team.team_id.clone()).collect();
    ids.sort();
    ids
}

fn agent_ids(agents: &[AgentRecord]) -> Vec<String> {
    let mut ids: Vec<String> = agents.iter().map(|agent| agent.agent_id.clone()).collect();
    ids.sort();
    ids
}

async fn insert_team(database: &Database, team_id: &str, carried: Option<&[String]>) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&team_id, &carried];
    let statement = r#"INSERT INTO "LiteLLM_TeamTable" ("team_id", "team_alias", "access_group_ids") VALUES ($1, $1, $2)"#;
    database.execute(statement, &parameters).await;
}

async fn insert_key(database: &Database, token: &str, carried: Option<&[String]>) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&token, &carried];
    let statement =
        r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "access_group_ids") VALUES ($1, $2)"#;
    database.execute(statement, &parameters).await;
}

async fn insert_agent(database: &Database, agent_id: &str, carried: Option<&[String]>) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&agent_id, &carried];
    let statement = r#"INSERT INTO "LiteLLM_AgentsTable" ("agent_id", "agent_name", "agent_card_params", "created_by", "updated_by", "access_group_ids") VALUES ($1, $1, '{}', 'seed', 'seed', $2)"#;
    database.execute(statement, &parameters).await;
}

async fn age_row(database: &Database, table: &str, key_column: &str, key: &str) {
    let statement = format!(
        r#"UPDATE "{table}" SET "updated_at" = TIMESTAMP '{SEEDED_AT}' WHERE "{key_column}" = $1"#
    );
    database.execute(&statement, &[&key]).await;
}

async fn updated_at(
    database: &Database,
    table: &str,
    key_column: &str,
    key: &str,
) -> NaiveDateTime {
    let statement = format!(r#"SELECT "updated_at" FROM "{table}" WHERE "{key_column}" = $1"#);
    let row = database.query_one(&statement, &[&key]).await;
    row.get(0)
}

#[tokio::test]
async fn a_created_group_reads_back_by_id_and_name() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    let created = tables
        .create(&new_group("math", Some("Math tools"), &["t1"]))
        .await
        .expect("the access group row inserts");
    assert_eq!(created.access_group_name, "math");
    assert_eq!(created.description.as_deref(), Some("Math tools"));
    assert_eq!(created.access_model_names, Some(groups(&["model-a"])));
    assert_eq!(created.access_mcp_server_ids, Some(Vec::new()));
    assert_eq!(created.assigned_team_ids, Some(groups(&["t1"])));
    assert_eq!(created.assigned_key_ids, Some(groups(&["token-a"])));
    assert_eq!(created.created_by.as_deref(), Some("creator"));
    assert_eq!(created.updated_by.as_deref(), Some("creator"));
    assert_eq!(created.created_at, created.updated_at);
    let parsed = Uuid::parse_str(&created.access_group_id).expect("the id is a UUID");
    assert_eq!(parsed.get_version_num(), 4_usize);
    let by_id = tables
        .litellm_accessgrouptable()
        .find_by_id(&created.access_group_id)
        .await
        .expect("the id query runs");
    assert_eq!(by_id, Some(created.clone()));
    let by_name = tables
        .litellm_accessgrouptable()
        .find_by_name("math")
        .await
        .expect("the name query runs");
    assert_eq!(by_name.as_ref(), Some(&created));
    assert_eq!(
        tables
            .find_by_name("absent")
            .await
            .expect("the name query runs"),
        None
    );
    assert_eq!(
        tables
            .find_by_id("absent")
            .await
            .expect("the id query runs"),
        None
    );
    database.close().await;
}

#[tokio::test]
async fn a_list_returns_the_newest_group_first() {
    let database = Database::start().await;
    let client = database.client().await;
    database
        .execute(
            r#"INSERT INTO "LiteLLM_AccessGroupTable" ("access_group_id", "access_group_name", "created_at", "updated_at") VALUES ('older', 'older', TIMESTAMP '2020-01-01 00:00:00', TIMESTAMP '2020-01-01 00:00:00'), ('newer', 'newer', TIMESTAMP '2021-01-01 00:00:00', TIMESTAMP '2021-01-01 00:00:00')"#,
            &[],
        )
        .await;
    let tables = PgAccessGroupTables { client: &client };
    let listed = tables
        .list_newest_first()
        .await
        .expect("the list query runs");
    let ids: Vec<String> = listed
        .iter()
        .map(|record| record.access_group_id.clone())
        .collect();
    assert_eq!(ids, vec!["newer".to_owned(), "older".to_owned()]);
    database.close().await;
}

#[tokio::test]
async fn an_update_can_clear_the_description() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    let created = tables
        .create(&new_group("math", Some("Math tools"), &["t1"]))
        .await
        .expect("the access group row inserts");
    age_row(
        &database,
        "LiteLLM_AccessGroupTable",
        "access_group_id",
        &created.access_group_id,
    )
    .await;
    let changes = AccessGroupChanges {
        access_group_name: None,
        description: FieldWrite::Null,
        access_model_names: None,
        access_mcp_server_ids: None,
        access_agent_ids: None,
        assigned_team_ids: None,
        assigned_key_ids: None,
        updated_by: Some("editor".to_owned()),
    };
    let updated = tables
        .update(&created.access_group_id, &changes)
        .await
        .expect("the access group row updates");
    assert_eq!(updated.description, None);
    assert_eq!(updated.access_group_name, "math");
    assert_eq!(updated.access_model_names, Some(groups(&["model-a"])));
    assert_eq!(updated.access_mcp_server_ids, Some(Vec::new()));
    assert_eq!(updated.assigned_team_ids, Some(groups(&["t1"])));
    assert_eq!(updated.assigned_key_ids, Some(groups(&["token-a"])));
    assert_eq!(updated.updated_by.as_deref(), Some("editor"));
    assert_eq!(updated.created_at, created.created_at);
    assert!(updated.updated_at > seeded_at());
    let missing = tables
        .update("absent", &changes)
        .await
        .expect_err("no row matches the id");
    assert!(matches!(missing, AccessGroupStoreError::RecordNotFound));
    database.close().await;
}

#[tokio::test]
async fn an_update_writes_only_the_columns_the_caller_set() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    let created = tables
        .create(&new_group("math", Some("Math tools"), &["t1"]))
        .await
        .expect("the access group row inserts");
    let changes = AccessGroupChanges {
        access_group_name: Some("renamed".to_owned()),
        description: FieldWrite::Unchanged,
        access_model_names: Some(groups(&["model-b"])),
        access_mcp_server_ids: None,
        access_agent_ids: None,
        assigned_team_ids: Some(Vec::new()),
        assigned_key_ids: None,
        updated_by: None,
    };
    let updated = tables
        .update(&created.access_group_id, &changes)
        .await
        .expect("the access group row updates");
    assert_eq!(updated.access_group_name, "renamed");
    assert_eq!(updated.description.as_deref(), Some("Math tools"));
    assert_eq!(updated.access_model_names, Some(groups(&["model-b"])));
    assert_eq!(updated.access_mcp_server_ids, Some(Vec::new()));
    assert_eq!(updated.assigned_team_ids, Some(Vec::new()));
    assert_eq!(updated.assigned_key_ids, Some(groups(&["token-a"])));
    assert_eq!(updated.updated_by, None);
    let renamed = tables
        .find_by_name("renamed")
        .await
        .expect("the name query runs")
        .expect("the renamed row");
    assert_eq!(renamed.access_group_id, created.access_group_id);
    assert_eq!(
        tables
            .find_by_name("math")
            .await
            .expect("the name query runs"),
        None
    );
    database.close().await;
}

#[tokio::test]
async fn a_duplicate_name_keeps_the_unique_violation() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    tables
        .create(&new_group("math", None, &[]))
        .await
        .expect("the first create runs");
    let error = tables
        .create(&new_group("math", None, &[]))
        .await
        .expect_err("the second create collides");
    let AccessGroupStoreError::Database(DbError::UniqueViolation { constraint, .. }) = error else {
        panic!("the second create reports a unique violation");
    };
    assert_eq!(
        constraint.as_deref(),
        Some("LiteLLM_AccessGroupTable_access_group_name_key")
    );
    database.close().await;
}

#[tokio::test]
async fn delete_removes_the_row_and_reports_a_missing_one() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    let created = tables
        .create(&new_group("math", None, &[]))
        .await
        .expect("the access group row inserts");
    tables
        .delete(&created.access_group_id)
        .await
        .expect("the row deletes");
    assert_eq!(
        tables
            .find_by_id(&created.access_group_id)
            .await
            .expect("the id query runs"),
        None
    );
    let missing = tables
        .delete(&created.access_group_id)
        .await
        .expect_err("the row is gone");
    assert!(matches!(missing, AccessGroupStoreError::RecordNotFound));
    database.close().await;
}

#[tokio::test]
async fn stored_nulls_read_as_none_on_every_record() {
    let database = Database::start().await;
    let client = database.client().await;
    database
        .execute(
            r#"INSERT INTO "LiteLLM_AccessGroupTable" ("access_group_id", "access_group_name", "access_model_names", "access_mcp_server_ids", "access_agent_ids", "assigned_team_ids", "assigned_key_ids") VALUES ('g-null', 'null-group', NULL, NULL, NULL, NULL, NULL)"#,
            &[],
        )
        .await;
    database
        .execute(
            r#"INSERT INTO "LiteLLM_TeamTable" ("team_id", "access_group_ids") VALUES ('t-null', NULL)"#,
            &[],
        )
        .await;
    database
        .execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "access_group_ids") VALUES ('k-null', NULL)"#,
            &[],
        )
        .await;
    database
        .execute(
            r#"INSERT INTO "LiteLLM_AgentsTable" ("agent_id", "agent_name", "agent_card_params", "created_by", "updated_by", "access_group_ids") VALUES ('a-null', 'null-agent', '{}', 'seed', 'seed', NULL)"#,
            &[],
        )
        .await;
    let tables = PgAccessGroupTables { client: &client };
    let group = tables
        .find_by_id("g-null")
        .await
        .expect("the id query runs")
        .expect("the group row");
    assert_eq!(group.description, None);
    assert_eq!(group.access_model_names, None);
    assert_eq!(group.access_mcp_server_ids, None);
    assert_eq!(group.access_agent_ids, None);
    assert_eq!(group.assigned_team_ids, None);
    assert_eq!(group.assigned_key_ids, None);
    let team = tables
        .litellm_teamtable()
        .find("t-null")
        .await
        .expect("the team query runs")
        .expect("the team row");
    assert_eq!(team.access_group_ids, None);
    let key = tables
        .litellm_verificationtoken()
        .find("k-null")
        .await
        .expect("the key query runs")
        .expect("the key row");
    assert_eq!(key.access_group_ids, None);
    let row = database
        .query_one(
            r#"SELECT "agent_id", "access_group_ids" FROM "LiteLLM_AgentsTable" WHERE "agent_id" = 'a-null'"#,
            &[],
        )
        .await;
    let agent = AgentRecord::from_row(&row).expect("the agent row decodes");
    assert_eq!(agent.access_group_ids, None);
    database.close().await;
}

#[tokio::test]
async fn team_filters_match_ids_carrying_groups_or_both() {
    let database = Database::start().await;
    let client = database.client().await;
    insert_team(&database, "t1", Some(&groups(&["g1"]))).await;
    insert_team(&database, "t2", Some(&groups(&["g2"]))).await;
    insert_team(&database, "t3", None).await;
    insert_team(&database, "t4", Some(&groups(&["g1", "g2"]))).await;
    let tables = PgAccessGroupTables { client: &client };
    let by_ids = tables
        .litellm_teamtable()
        .find_many(&TeamWhere::IdIn(groups(&["t1", "t3"])))
        .await
        .expect("the id filter runs");
    assert_eq!(team_ids(&by_ids), groups(&["t1", "t3"]));
    let carrying = tables
        .litellm_teamtable()
        .find_many(&TeamWhere::CarryingAny(groups(&["g2"])))
        .await
        .expect("the carrying filter runs");
    assert_eq!(team_ids(&carrying), groups(&["t2", "t4"]));
    let either = tables
        .litellm_teamtable()
        .find_many(&TeamWhere::CarryingAnyOrIdIn {
            groups: groups(&["g2"]),
            ids: groups(&["t1"]),
        })
        .await
        .expect("the either filter runs");
    assert_eq!(team_ids(&either), groups(&["t1", "t2", "t4"]));
    database.close().await;
}

#[tokio::test]
async fn an_empty_team_filter_matches_no_rows() {
    let database = Database::start().await;
    let client = database.client().await;
    insert_team(&database, "t1", Some(&groups(&["g1"]))).await;
    let tables = PgAccessGroupTables { client: &client };
    let empty: Vec<String> = Vec::new();
    let by_ids = tables
        .litellm_teamtable()
        .find_many(&TeamWhere::IdIn(empty.clone()))
        .await
        .expect("the id filter runs");
    assert!(by_ids.is_empty());
    let carrying = tables
        .litellm_teamtable()
        .find_many(&TeamWhere::CarryingAny(empty.clone()))
        .await
        .expect("the carrying filter runs");
    assert!(carrying.is_empty());
    let either = tables
        .litellm_teamtable()
        .find_many(&TeamWhere::CarryingAnyOrIdIn {
            groups: empty.clone(),
            ids: empty,
        })
        .await
        .expect("the either filter runs");
    assert!(either.is_empty());
    database.close().await;
}

#[tokio::test]
async fn key_reads_find_one_token_or_every_key_carrying_a_group() {
    let database = Database::start().await;
    let client = database.client().await;
    insert_key(&database, "token-1", Some(&groups(&["g1", "g2"]))).await;
    insert_key(&database, "token-2", Some(&groups(&["g2"]))).await;
    insert_key(&database, "token-3", None).await;
    let tables = PgAccessGroupTables { client: &client };
    let found = tables
        .litellm_verificationtoken()
        .find("token-1")
        .await
        .expect("the token query runs")
        .expect("the key row");
    assert_eq!(found.access_group_ids, Some(groups(&["g1", "g2"])));
    assert_eq!(
        tables
            .litellm_verificationtoken()
            .find("missing")
            .await
            .expect("the token query runs"),
        None
    );
    let carrying = tables
        .litellm_verificationtoken()
        .find_carrying("g1")
        .await
        .expect("the carrying query runs");
    let tokens: Vec<String> = carrying.iter().map(|key| key.token.clone()).collect();
    assert_eq!(tokens, groups(&["token-1"]));
    database.close().await;
}

#[tokio::test]
async fn agent_reads_find_every_agent_carrying_a_group() {
    let database = Database::start().await;
    let client = database.client().await;
    insert_agent(&database, "agent-1", Some(&groups(&["g1"]))).await;
    insert_agent(&database, "agent-2", Some(&groups(&["g2"]))).await;
    insert_agent(&database, "agent-3", None).await;
    let tables = PgAccessGroupTables { client: &client };
    let carrying = tables
        .litellm_agentstable()
        .find_carrying("g1")
        .await
        .expect("the carrying query runs");
    assert_eq!(agent_ids(&carrying), groups(&["agent-1"]));
    database.close().await;
}

#[tokio::test]
async fn membership_writes_store_the_list_and_move_updated_at() {
    let database = Database::start().await;
    let client = database.client().await;
    insert_team(&database, "t1", Some(&groups(&["g1"]))).await;
    insert_key(&database, "token-1", Some(&groups(&["g1"]))).await;
    insert_agent(&database, "agent-1", Some(&groups(&["g1"]))).await;
    age_row(&database, "LiteLLM_TeamTable", "team_id", "t1").await;
    age_row(&database, "LiteLLM_VerificationToken", "token", "token-1").await;
    age_row(&database, "LiteLLM_AgentsTable", "agent_id", "agent-1").await;
    let replacement = groups(&["g2", "g3"]);
    let tables = PgAccessGroupTables { client: &client };
    tables
        .litellm_teamtable()
        .set_access_group_ids("t1", &replacement)
        .await
        .expect("the team write runs");
    tables
        .litellm_verificationtoken()
        .set_access_group_ids("token-1", &replacement)
        .await
        .expect("the key write runs");
    tables
        .litellm_agentstable()
        .set_access_group_ids("agent-1", &replacement)
        .await
        .expect("the agent write runs");
    let team = tables
        .litellm_teamtable()
        .find("t1")
        .await
        .expect("the team query runs")
        .expect("the team row");
    assert_eq!(team.access_group_ids, Some(replacement.clone()));
    assert_eq!(team.team_alias.as_deref(), Some("t1"));
    let key = tables
        .litellm_verificationtoken()
        .find("token-1")
        .await
        .expect("the key query runs")
        .expect("the key row");
    assert_eq!(key.access_group_ids, Some(replacement.clone()));
    let carrying = tables
        .litellm_agentstable()
        .find_carrying("g2")
        .await
        .expect("the carrying query runs");
    assert_eq!(agent_ids(&carrying), groups(&["agent-1"]));
    assert!(updated_at(&database, "LiteLLM_TeamTable", "team_id", "t1").await > seeded_at());
    assert!(
        updated_at(&database, "LiteLLM_VerificationToken", "token", "token-1").await > seeded_at()
    );
    assert!(
        updated_at(&database, "LiteLLM_AgentsTable", "agent_id", "agent-1").await > seeded_at()
    );
    database.close().await;
}

#[tokio::test]
async fn membership_writes_report_a_missing_row() {
    let database = Database::start().await;
    let client = database.client().await;
    let tables = PgAccessGroupTables { client: &client };
    let replacement = groups(&["g2"]);
    let team = tables
        .litellm_teamtable()
        .set_access_group_ids("absent", &replacement)
        .await
        .expect_err("no team matches the id");
    assert!(matches!(team, AccessGroupStoreError::RecordNotFound));
    let key = tables
        .litellm_verificationtoken()
        .set_access_group_ids("absent", &replacement)
        .await
        .expect_err("no key matches the token");
    assert!(matches!(key, AccessGroupStoreError::RecordNotFound));
    let agent = tables
        .litellm_agentstable()
        .set_access_group_ids("absent", &replacement)
        .await
        .expect_err("no agent matches the id");
    assert!(matches!(agent, AccessGroupStoreError::RecordNotFound));
    database.close().await;
}

#[tokio::test]
async fn a_rolled_back_transaction_leaves_every_row_as_it_was() {
    let database = Database::start().await;
    insert_team(&database, "t1", Some(&groups(&["g1"]))).await;
    age_row(&database, "LiteLLM_TeamTable", "team_id", "t1").await;
    let mut client = database.client().await;
    let transaction = client.transaction().await.expect("the transaction opens");
    {
        let tables = PgAccessGroupTables {
            client: &transaction,
        };
        tables
            .create(&new_group("rolled-back", None, &[]))
            .await
            .expect("the insert runs");
        tables
            .litellm_teamtable()
            .set_access_group_ids("t1", &groups(&["g2"]))
            .await
            .expect("the team write runs");
    }
    transaction
        .rollback()
        .await
        .expect("the transaction rolls back");
    let tables = PgAccessGroupTables { client: &client };
    assert_eq!(
        tables
            .find_by_name("rolled-back")
            .await
            .expect("the name query runs"),
        None
    );
    let team = tables
        .litellm_teamtable()
        .find("t1")
        .await
        .expect("the team query runs")
        .expect("the team row");
    assert_eq!(team.access_group_ids, Some(groups(&["g1"])));
    assert_eq!(
        updated_at(&database, "LiteLLM_TeamTable", "team_id", "t1").await,
        seeded_at()
    );
    database.close().await;
}

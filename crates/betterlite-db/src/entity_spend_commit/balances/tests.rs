#![cfg(test)]

use std::time::Duration;

use chrono::NaiveDateTime;
use rstest::rstest;

use crate::entity_spend_commit::balances::PostgresEntityTables;
use crate::entity_spend_commit::progress::EntityCommitError;
use crate::entity_spend_commit::tests::support::{
    Database, MEMBER_SPEND, MEMBER_TOTAL_SPEND, Ports, SEED_SQL, amount, commit_through,
    flush_moment, increments, transactions,
};
use crate::entity_spend_commit::{EntityTableError, EntityTableWriter as _};
use crate::postgres_store::session::PostgresStore;
use crate::spend_commit::policy::{SpendFailure, SpendUpdateFailure};
use crate::spend_commit::tables::SpendTable;
use crate::spend_commit::transaction::SpendDeadline;

const USER_SPEND: &str = r#"SELECT spend::numeric FROM "LiteLLM_UserTable" WHERE user_id = $1"#;
const KEY_SPEND: &str =
    r#"SELECT spend::numeric FROM "LiteLLM_VerificationToken" WHERE token = $1"#;
const KEY_TOTAL_SPEND: &str =
    r#"SELECT total_spend::numeric FROM "LiteLLM_VerificationToken" WHERE token = $1"#;
const END_USER_SPEND: &str =
    r#"SELECT spend::numeric FROM "LiteLLM_EndUserTable" WHERE user_id = $1"#;

#[rstest]
#[case::users(SpendTable::UserList, USER_SPEND, "u1", "u2")]
#[case::keys(SpendTable::KeyList, KEY_SPEND, "k1", "k2")]
#[case::teams(
    SpendTable::TeamList,
    r#"SELECT spend::numeric FROM "LiteLLM_TeamTable" WHERE team_id = $1"#,
    "t1",
    "t2"
)]
#[case::orgs(
    SpendTable::OrgList,
    r#"SELECT spend::numeric FROM "LiteLLM_OrganizationTable" WHERE organization_id = $1"#,
    "o 1",
    "o2"
)]
#[case::org_members(
    SpendTable::OrgMemberList,
    r#"SELECT spend::numeric FROM "LiteLLM_OrganizationMembership" WHERE 'organization_id::' || replace(organization_id, ' ', '%20') || '::user_id::' || replace(user_id, ' ', '%20') = $1"#,
    "organization_id::o%201::user_id::u%201",
    "organization_id::o%201::user_id::u1"
)]
#[case::projects(
    SpendTable::ProjectList,
    r#"SELECT spend::numeric FROM "LiteLLM_ProjectTable" WHERE project_id = $1"#,
    "p1",
    "p2"
)]
#[case::tags(
    SpendTable::TagList,
    r#"SELECT spend::numeric FROM "LiteLLM_TagTable" WHERE tag_name = $1"#,
    "tag1",
    "tag2"
)]
#[case::access_groups(
    SpendTable::ModelAccessGroupList,
    r#"SELECT spend::numeric FROM "LiteLLM_ModelAccessGroupBudgetTable" WHERE access_group_name = $1"#,
    "g1",
    "g2"
)]
#[case::agents(
    SpendTable::AgentList,
    r#"SELECT spend::numeric FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#,
    "a1",
    "a2"
)]
#[tokio::test]
async fn existing_rows_increment_and_missing_rows_stay_missing(
    #[case] table: SpendTable,
    #[case] spend_sql: &str,
    #[case] existing: &str,
    #[case] missing: &str,
) {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let rows = increments(&[(existing, "0.5"), (missing, "0.25")]);
    writer.write_table(table, &rows).await.unwrap();
    assert_eq!(database.spend(spend_sql, existing).await, amount("1.5"));
    assert_eq!(database.spend(spend_sql, missing).await, None);
    database.close().await;
}

#[tokio::test]
async fn end_users_are_created_unblocked_then_incremented() {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let rows = increments(&[("e1", "0.5"), ("e2", "0.25")]);
    writer
        .write_table(SpendTable::EndUserList, &rows)
        .await
        .unwrap();
    assert_eq!(database.spend(END_USER_SPEND, "e1").await, amount("1.5"));
    assert_eq!(database.spend(END_USER_SPEND, "e2").await, amount("0.25"));
    writer
        .write_table(SpendTable::EndUserList, &rows)
        .await
        .unwrap();
    assert_eq!(database.spend(END_USER_SPEND, "e2").await, amount("0.5"));
    let blocked: bool = database
        .client
        .query_one(
            r#"SELECT blocked FROM "LiteLLM_EndUserTable" WHERE user_id = 'e2'"#,
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!blocked);
    database.close().await;
}

#[tokio::test]
async fn keys_advance_spend_total_spend_and_last_active() {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let rows = increments(&[("k1", "0.5")]);
    writer
        .write_table(SpendTable::KeyList, &rows)
        .await
        .unwrap();
    assert_eq!(database.spend(KEY_SPEND, "k1").await, amount("1.5"));
    assert_eq!(database.spend(KEY_TOTAL_SPEND, "k1").await, amount("3.5"));
    let last_active: NaiveDateTime = database
        .client
        .query_one(
            r#"SELECT last_active FROM "LiteLLM_VerificationToken" WHERE token = 'k1'"#,
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(last_active, flush_moment());
    database.close().await;
}

#[tokio::test]
async fn zero_then_fractional_costs_share_one_connection() {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    write_member_and_user(&writer, "0").await;
    write_member_and_user(&writer, "0.5").await;
    assert_eq!(database.spend(MEMBER_SPEND, "t1/u1").await, amount("1.5"));
    assert_eq!(
        database.spend(MEMBER_TOTAL_SPEND, "t1/u1").await,
        amount("3.5")
    );
    assert_eq!(database.spend(USER_SPEND, "u1").await, amount("1.5"));
    database.close().await;
}

// The same cost through a team membership and a user row, over the writer's one pooled connection.
async fn write_member_and_user(writer: &PostgresEntityTables<'_>, cost: &str) {
    let members = increments(&[("team_id::t1::user_id::u1", cost)]);
    writer
        .write_table(SpendTable::TeamMemberList, &members)
        .await
        .unwrap();
    let users = increments(&[("u1", cost)]);
    writer
        .write_table(SpendTable::UserList, &users)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_unreachable_database_is_tried_n_plus_one_times() {
    let ports = Ports::new();
    let writer = ports.tables(PostgresStore { pool: None });
    let input = transactions(&[(SpendTable::UserList, &[("u1", "1")])]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    let error = outcome.unwrap_err();
    assert_eq!(error.spend_failure(), SpendFailure::NotSent);
    assert!(committed.is_empty());
    assert_eq!(
        ports.pause.delays(),
        [Duration::from_secs(1), Duration::from_secs(2)]
    );
    assert_eq!(ports.failures.count(), 1);
}

#[tokio::test]
async fn a_refused_statement_is_reported_without_a_retry() {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    database
        .seed(r#"ALTER TABLE "LiteLLM_UserTable" ADD CONSTRAINT spend_cap CHECK (spend <= 100)"#)
        .await;
    let ports = Ports::new();
    let writer = ports.tables(database.store.clone());
    let input = transactions(&[(SpendTable::UserList, &[("u 1", "1"), ("u1", "500")])]);
    let (committed, outcome) = commit_through(&ports, &writer, &ports.cache(), &input).await;
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        EntityCommitError::Write(EntityTableError::Statement { .. })
    ));
    assert_eq!(error.spend_failure(), SpendFailure::Rejected);
    assert!(committed.is_empty());
    assert!(ports.pause.delays().is_empty());
    assert_eq!(ports.failures.count(), 1);
    assert_eq!(database.spend(USER_SPEND, "u 1").await, amount("1"));
    database.close().await;
}

#[tokio::test]
async fn a_statement_timeout_leaves_nothing_applied() {
    let mut database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    let mut holder = database.session().await;
    let hold = holder.transaction().await.unwrap();
    hold.batch_execute(r#"SELECT 1 FROM "LiteLLM_UserTable" WHERE user_id = 'u1' FOR UPDATE"#)
        .await
        .unwrap();
    let ports = Ports::new();
    let mut writer = ports.tables(database.store.clone());
    writer.deadline = SpendDeadline {
        duration: Duration::from_millis(300),
    };
    let rows = increments(&[("u1", "1")]);
    let error = writer
        .write_table(SpendTable::UserList, &rows)
        .await
        .unwrap_err();
    assert!(matches!(error, EntityTableError::Timeout { .. }));
    assert_eq!(error.spend_failure(), SpendFailure::Other);
    hold.rollback().await.unwrap();
    assert_eq!(database.spend(USER_SPEND, "u1").await, amount("1"));
    database.close().await;
}

#[tokio::test]
async fn a_commit_timeout_is_ambiguous() {
    let database = Database::open(1).await;
    database.seed(SEED_SQL).await;
    database
        .seed(
            r#"CREATE FUNCTION slow_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(2); RETURN NULL; END $$;
CREATE CONSTRAINT TRIGGER slow_commit AFTER UPDATE ON "LiteLLM_UserTable" DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION slow_commit();"#,
        )
        .await;
    let ports = Ports::new();
    let mut writer = ports.tables(database.store.clone());
    writer.deadline = SpendDeadline {
        duration: Duration::from_millis(500),
    };
    let rows = increments(&[("u1", "1")]);
    let error = writer
        .write_table(SpendTable::UserList, &rows)
        .await
        .unwrap_err();
    assert!(matches!(error, EntityTableError::CommitTimeout { .. }));
    assert_eq!(error.spend_failure(), SpendFailure::Ambiguous);
    database.close().await;
}

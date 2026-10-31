#![cfg(test)]

use super::{BASELINE_MIGRATION, MIGRATION_CATALOG};
use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use std::str;
use tokio::task::JoinSet;
use tokio_postgres::Client;
use tokio_postgres::error::SqlState;

const PINNED_SHA256: &str = "0bf3d5cabe50d5d7249eb505cf2ec6fe2d8b49dc8b496dceb2179a357ed4725e";

fn migration_sql() -> &'static str {
    str::from_utf8(BASELINE_MIGRATION.bytes).unwrap()
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    Environment::parse_from(["schema-baseline"])
        .url
        .map(SecretString::from)
}

type ConnectionTasks = JoinSet<Result<(), tokio_postgres::Error>>;

async fn setup() -> (PostgresFixture, Client, ConnectionTasks) {
    let url = postgres_test_url();
    let mut generator = RandomDatabaseName;
    let fixture = PostgresFixture::provision_with_name_generator(
        url.as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    )
    .resolve()
    .await
    .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .unwrap();
    (fixture, client, tasks)
}

async fn teardown(client: Client, mut tasks: ConnectionTasks, fixture: PostgresFixture) {
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
    fixture.close_async().await.unwrap();
}

async fn exec(client: &Client, sql: &str) {
    client.execute(sql, &[]).await.unwrap();
}

async fn scalar_text(client: &Client, sql: &str) -> String {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

async fn scalar_i64(client: &Client, sql: &str) -> i64 {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

async fn insert_budget(client: &Client, id: &str) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ($1, 't', 't')"#,
            &[&id],
        )
        .await
        .unwrap();
}

struct ColumnInfo {
    data_type: String,
    is_nullable: bool,
    default: Option<String>,
}

async fn column(client: &Client, table: &str, name: &str) -> ColumnInfo {
    let row = client
        .query_one(
            "SELECT data_type, is_nullable, column_default FROM information_schema.columns \
             WHERE table_name = $1 AND column_name = $2",
            &[&table, &name],
        )
        .await
        .unwrap();
    ColumnInfo {
        data_type: row.get(0),
        is_nullable: row.get::<_, String>(1) == "YES",
        default: row.get(2),
    }
}

async fn index_names(client: &Client, table: &str) -> Vec<String> {
    client
        .query(
            "SELECT indexname FROM pg_indexes WHERE tablename = $1",
            &[&table],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

async fn assert_rule(client: &Client, conname: &str, delete_rule: char) {
    let row = client
        .query_one(
            "SELECT confupdtype, confdeltype FROM pg_constraint WHERE conname = $1",
            &[&conname],
        )
        .await
        .unwrap();
    let update: i8 = row.get(0);
    let delete: i8 = row.get(1);
    assert_eq!(char::from(u8::try_from(update).unwrap()), 'c');
    assert_eq!(char::from(u8::try_from(delete).unwrap()), delete_rule);
}

fn is_zero(default: Option<String>) -> bool {
    let value: f64 = default.unwrap().trim().parse().unwrap();
    value.abs() < f64::EPSILON
}

#[test]
fn baseline_sql_identity() {
    let mut hasher = Sha256::new();
    hasher.update(BASELINE_MIGRATION.bytes);
    assert_eq!(hex::encode(hasher.finalize()), PINNED_SHA256);
    assert_eq!(BASELINE_MIGRATION.sha256, PINNED_SHA256);
    assert_eq!(BASELINE_MIGRATION.directory, "20250326162113_baseline");
    assert_eq!(BASELINE_MIGRATION.bytes.len(), 15_827);
    let sql = migration_sql();
    assert_eq!(sql.matches("CREATE TABLE IF NOT EXISTS").count(), 17);
    let regular_indexes = sql.matches("CREATE INDEX IF NOT EXISTS").count();
    let unique_indexes = sql.matches("CREATE UNIQUE INDEX IF NOT EXISTS").count();
    assert_eq!(regular_indexes + unique_indexes, 6);
    assert_eq!(sql.matches("ADD CONSTRAINT").count(), 14);
    assert!(sql.ends_with('\n'));
}

#[test]
fn original_directory_registration() {
    assert_eq!(MIGRATION_CATALOG.files, &[BASELINE_MIGRATION]);
}

#[test]
fn statement_order_tables_indexes_foreign_keys() {
    let sql = migration_sql();
    let (before_indexes, after_indexes) = sql.split_once("-- CreateIndex").unwrap();
    let (indexes, _) = after_indexes.split_once("-- AddForeignKey").unwrap();
    assert!(before_indexes.contains("-- CreateTable"));
    assert!(!indexes.is_empty());
}

#[test]
fn seventeen_tables_six_indexes_fourteen_foreign_keys() {
    let sql = migration_sql();
    assert_eq!(sql.matches("-- CreateTable").count(), 17);
    assert_eq!(sql.matches("-- CreateIndex").count(), 6);
    assert_eq!(sql.matches("-- AddForeignKey").count(), 14);
}

#[tokio::test]
async fn baseline_defaults() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let created_at = column(&client, "LiteLLM_BudgetTable", "created_at").await;
    assert_eq!(created_at.default.unwrap(), "CURRENT_TIMESTAMP");
    let teams = column(&client, "LiteLLM_UserTable", "teams").await;
    assert!(teams.default.is_some());
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_budget_nullable_limits_and_required_actors() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let max_budget = column(&client, "LiteLLM_BudgetTable", "max_budget").await;
    assert_eq!(max_budget.data_type, "double precision");
    assert!(max_budget.is_nullable);
    assert_eq!(max_budget.default, None);
    let created_by = column(&client, "LiteLLM_BudgetTable", "created_by").await;
    assert_eq!(created_by.data_type, "text");
    assert!(!created_by.is_nullable);
    assert_eq!(created_by.default, None);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_credentials_required_values_nullable_info() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let values = column(&client, "LiteLLM_CredentialsTable", "credential_values").await;
    assert_eq!(values.data_type, "jsonb");
    assert!(!values.is_nullable);
    let info = column(&client, "LiteLLM_CredentialsTable", "credential_info").await;
    assert_eq!(info.data_type, "jsonb");
    assert!(info.is_nullable);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_proxy_model_required_params_nullable_info() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let params = column(&client, "LiteLLM_ProxyModelTable", "litellm_params").await;
    assert_eq!(params.data_type, "jsonb");
    assert!(!params.is_nullable);
    let info = column(&client, "LiteLLM_ProxyModelTable", "model_info").await;
    assert!(info.is_nullable);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_organization_json_spend_and_nullable_models() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let models = column(&client, "LiteLLM_OrganizationTable", "models").await;
    assert_eq!(models.data_type, "ARRAY");
    assert!(models.is_nullable);
    assert_eq!(models.default, None);
    let spend = column(&client, "LiteLLM_OrganizationTable", "spend").await;
    assert!(!spend.is_nullable);
    assert!(is_zero(spend.default));
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_serial_model_id_and_nullable_aliases() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let id = column(&client, "LiteLLM_ModelTable", "id").await;
    assert_eq!(id.data_type, "integer");
    assert!(!id.is_nullable);
    assert!(id.default.unwrap().starts_with("nextval("));
    let aliases = column(&client, "LiteLLM_ModelTable", "aliases").await;
    assert_eq!(aliases.data_type, "jsonb");
    assert!(aliases.is_nullable);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_team_object_roles_false_blocked_and_nullable_arrays() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let roles = column(&client, "LiteLLM_TeamTable", "members_with_roles").await;
    assert_eq!(roles.data_type, "jsonb");
    assert!(!roles.is_nullable);
    assert!(roles.default.unwrap().contains("{}"));
    let blocked = column(&client, "LiteLLM_TeamTable", "blocked").await;
    assert_eq!(blocked.data_type, "boolean");
    assert!(!blocked.is_nullable);
    assert!(blocked.default.unwrap().contains("false"));
    let models = column(&client, "LiteLLM_TeamTable", "models").await;
    assert_eq!(models.data_type, "ARRAY");
    assert!(models.is_nullable);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_user_nullable_empty_arrays_and_timestamps() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let teams = column(&client, "LiteLLM_UserTable", "teams").await;
    assert_eq!(teams.data_type, "ARRAY");
    assert!(teams.is_nullable);
    assert!(teams.default.is_some());
    let created_at = column(&client, "LiteLLM_UserTable", "created_at").await;
    assert!(created_at.is_nullable);
    assert!(created_at.default.is_some());
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_key_six_objects_false_cooldown_and_nullable_blocked() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let cooldown = column(&client, "LiteLLM_VerificationToken", "soft_budget_cooldown").await;
    assert_eq!(cooldown.data_type, "boolean");
    assert!(!cooldown.is_nullable);
    assert!(cooldown.default.unwrap().contains("false"));
    let aliases = column(&client, "LiteLLM_VerificationToken", "aliases").await;
    assert_eq!(aliases.data_type, "jsonb");
    assert!(!aliases.is_nullable);
    assert!(aliases.default.unwrap().contains("{}"));
    let config = column(&client, "LiteLLM_VerificationToken", "config").await;
    assert_eq!(config.data_type, "jsonb");
    assert!(!config.is_nullable);
    assert!(config.default.unwrap().contains("{}"));
    let permissions = column(&client, "LiteLLM_VerificationToken", "permissions").await;
    assert_eq!(permissions.data_type, "jsonb");
    assert!(!permissions.is_nullable);
    assert!(permissions.default.unwrap().contains("{}"));
    let metadata = column(&client, "LiteLLM_VerificationToken", "metadata").await;
    assert_eq!(metadata.data_type, "jsonb");
    assert!(!metadata.is_nullable);
    assert!(metadata.default.unwrap().contains("{}"));
    let model_spend = column(&client, "LiteLLM_VerificationToken", "model_spend").await;
    assert_eq!(model_spend.data_type, "jsonb");
    assert!(!model_spend.is_nullable);
    assert!(model_spend.default.unwrap().contains("{}"));
    let model_max_budget = column(&client, "LiteLLM_VerificationToken", "model_max_budget").await;
    assert_eq!(model_max_budget.data_type, "jsonb");
    assert!(!model_max_budget.is_nullable);
    assert!(model_max_budget.default.unwrap().contains("{}"));
    let blocked = column(&client, "LiteLLM_VerificationToken", "blocked").await;
    assert!(blocked.is_nullable);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_end_user_false_blocked() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let blocked = column(&client, "LiteLLM_EndUserTable", "blocked").await;
    assert_eq!(blocked.data_type, "boolean");
    assert!(!blocked.is_nullable);
    assert!(blocked.default.unwrap().contains("false"));
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_config_nullable_json() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let value = column(&client, "LiteLLM_Config", "param_value").await;
    assert_eq!(value.data_type, "jsonb");
    assert!(value.is_nullable);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_spend_log_empty_strings_json_arrays_and_required_times() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let api_key = column(&client, "LiteLLM_SpendLogs", "api_key").await;
    assert!(!api_key.is_nullable);
    assert!(api_key.default.unwrap().contains("''"));
    let request_tags = column(&client, "LiteLLM_SpendLogs", "request_tags").await;
    assert_eq!(request_tags.data_type, "jsonb");
    assert!(request_tags.default.unwrap().contains("[]"));
    let start_time = column(&client, "LiteLLM_SpendLogs", "startTime").await;
    assert!(!start_time.is_nullable);
    assert_eq!(start_time.default, None);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_error_log_text_status_and_required_times() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let status = column(&client, "LiteLLM_ErrorLogs", "status_code").await;
    assert_eq!(status.data_type, "text");
    assert!(!status.is_nullable);
    assert!(status.default.unwrap().contains("''"));
    let end_time = column(&client, "LiteLLM_ErrorLogs", "endTime").await;
    assert!(!end_time.is_nullable);
    assert_eq!(end_time.default, None);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_notification_required_status_nullable_models() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let status = column(&client, "LiteLLM_UserNotifications", "status").await;
    assert!(!status.is_nullable);
    let models = column(&client, "LiteLLM_UserNotifications", "models").await;
    assert_eq!(models.data_type, "ARRAY");
    assert!(models.is_nullable);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_team_membership_required_spend() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let spend = column(&client, "LiteLLM_TeamMembership", "spend").await;
    assert!(!spend.is_nullable);
    assert!(is_zero(spend.default));
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_organization_membership_nullable_defaults() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let spend = column(&client, "LiteLLM_OrganizationMembership", "spend").await;
    assert!(spend.is_nullable);
    assert!(is_zero(spend.default));
    let created_at = column(&client, "LiteLLM_OrganizationMembership", "created_at").await;
    assert!(created_at.is_nullable);
    assert!(created_at.default.is_some());
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_invitation_no_timestamp_defaults() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let expires_at = column(&client, "LiteLLM_InvitationLink", "expires_at").await;
    assert!(!expires_at.is_nullable);
    assert_eq!(expires_at.default, None);
    let created_at = column(&client, "LiteLLM_InvitationLink", "created_at").await;
    assert!(!created_at.is_nullable);
    assert_eq!(created_at.default, None);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_defaults_audit_empty_actor_defaults() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    let changed_by = column(&client, "LiteLLM_AuditLog", "changed_by").await;
    assert!(!changed_by.is_nullable);
    assert!(changed_by.default.unwrap().contains("''"));
    let changed_by_api_key = column(&client, "LiteLLM_AuditLog", "changed_by_api_key").await;
    assert!(!changed_by_api_key.is_nullable);
    assert!(changed_by_api_key.default.unwrap().contains("''"));
    teardown(client, tasks, fixture).await;
}

type ColumnFacts = (i32, String, String, bool, bool);

async fn table_columns(client: &Client, schema: &str, table: &str) -> Vec<ColumnFacts> {
    client
        .query(
            "SELECT ordinal_position, column_name, data_type, is_nullable = 'YES', column_default IS NOT NULL \
             FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
            &[&schema, &table],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)))
        .collect()
}

async fn primary_key_columns(client: &Client, schema: &str, table: &str) -> Vec<String> {
    client
        .query(
            "SELECT kcu.column_name FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
             ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
             WHERE tc.table_schema = $1 AND tc.table_name = $2 AND tc.constraint_type = 'PRIMARY KEY' \
             ORDER BY kcu.ordinal_position",
            &[&schema, &table],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[rstest]
#[case::budget_table("LiteLLM_BudgetTable")]
#[case::credentials_table("LiteLLM_CredentialsTable")]
#[case::proxy_model_table("LiteLLM_ProxyModelTable")]
#[case::organization_table("LiteLLM_OrganizationTable")]
#[case::model_table("LiteLLM_ModelTable")]
#[case::team_table("LiteLLM_TeamTable")]
#[case::user_table("LiteLLM_UserTable")]
#[case::verification_token("LiteLLM_VerificationToken")]
#[case::end_user_table("LiteLLM_EndUserTable")]
#[case::config("LiteLLM_Config")]
#[case::spend_logs("LiteLLM_SpendLogs")]
#[case::error_logs("LiteLLM_ErrorLogs")]
#[case::user_notifications("LiteLLM_UserNotifications")]
#[case::team_membership("LiteLLM_TeamMembership")]
#[case::organization_membership("LiteLLM_OrganizationMembership")]
#[case::invitation_link("LiteLLM_InvitationLink")]
#[case::audit_log("LiteLLM_AuditLog")]
#[tokio::test]
async fn baseline_catalog_columns(#[case] table: &str) {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    client
        .batch_execute("CREATE SCHEMA upstream;")
        .await
        .unwrap();
    client
        .batch_execute(&format!(
            "SET search_path TO upstream; {}",
            include_str!("../../../../tools/postgres-fixture/fixtures/baseline.sql")
        ))
        .await
        .unwrap();
    let candidate = table_columns(&client, "public", table).await;
    let upstream = table_columns(&client, "upstream", table).await;
    assert_eq!(candidate, upstream, "{table} columns differ from upstream");
    assert!(!candidate.is_empty(), "{table} has no columns");
    let candidate_pk = primary_key_columns(&client, "public", table).await;
    let upstream_pk = primary_key_columns(&client, "upstream", table).await;
    assert_eq!(
        candidate_pk, upstream_pk,
        "{table} primary key differs from upstream"
    );
    assert!(!candidate_pk.is_empty(), "{table} has no primary key");
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_constraints() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();

    exec(
        &client,
        r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_id, credential_name, credential_values, created_by, updated_by) VALUES ('c1', 'dup', '{}', 't', 't')"#,
    )
    .await;
    let error = client
        .execute(
            r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_id, credential_name, credential_values, created_by, updated_by) VALUES ('c2', 'dup', '{}', 't', 't')"#,
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), Some(&SqlState::UNIQUE_VIOLATION));

    exec(
        &client,
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('t1')"#,
    )
    .await;
    exec(
        &client,
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('t2')"#,
    )
    .await;
    exec(
        &client,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1')"#,
    )
    .await;
    exec(
        &client,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u2')"#,
    )
    .await;

    let spend_log_indexes = index_names(&client, "LiteLLM_SpendLogs").await;
    assert!(spend_log_indexes.contains(&"LiteLLM_SpendLogs_startTime_idx".to_owned()));
    assert!(spend_log_indexes.contains(&"LiteLLM_SpendLogs_end_user_idx".to_owned()));

    let membership_indexes = index_names(&client, "LiteLLM_OrganizationMembership").await;
    assert_eq!(membership_indexes.len(), 2);

    teardown(client, tasks, fixture).await;
}

struct FkCase {
    conname: &'static str,
    delete_rule: char,
    setup_sql: &'static str,
    update_sql: &'static str,
    read_sql: &'static str,
    delete_sql: &'static str,
    expected_after_update: &'static str,
}

#[rstest]
#[case::organization_table_budget_id_fkey(FkCase {
    conname: "LiteLLM_OrganizationTable_budget_id_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'b2' WHERE budget_id = 'b1';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b2';"#,
    expected_after_update: "b2",
})]
#[case::team_table_organization_id_fkey(FkCase {
    conname: "LiteLLM_TeamTable_organization_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_TeamTable" (team_id, organization_id) VALUES ('t1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_OrganizationTable" SET organization_id = 'o2' WHERE organization_id = 'o1';"#,
    read_sql: r#"SELECT organization_id FROM "LiteLLM_TeamTable" WHERE team_id = 't1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o2';"#,
    expected_after_update: "o2",
})]
#[case::team_table_model_id_fkey(FkCase {
    conname: "LiteLLM_TeamTable_model_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_ModelTable" (id, created_by, updated_by) VALUES (9001, 't', 't');
       INSERT INTO "LiteLLM_TeamTable" (team_id, model_id) VALUES ('t1', 9001);"#,
    update_sql: r#"UPDATE "LiteLLM_ModelTable" SET id = 9002 WHERE id = 9001;"#,
    read_sql: r#"SELECT model_id::TEXT FROM "LiteLLM_TeamTable" WHERE team_id = 't1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_ModelTable" WHERE id = 9002;"#,
    expected_after_update: "9002",
})]
#[case::user_table_organization_id_fkey(FkCase {
    conname: "LiteLLM_UserTable_organization_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_UserTable" (user_id, organization_id) VALUES ('u1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_OrganizationTable" SET organization_id = 'o2' WHERE organization_id = 'o1';"#,
    read_sql: r#"SELECT organization_id FROM "LiteLLM_UserTable" WHERE user_id = 'u1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o2';"#,
    expected_after_update: "o2",
})]
#[case::verification_token_budget_id_fkey(FkCase {
    conname: "LiteLLM_VerificationToken_budget_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_VerificationToken" (token, budget_id) VALUES ('tok1', 'b1');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'b2' WHERE budget_id = 'b1';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_VerificationToken" WHERE token = 'tok1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b2';"#,
    expected_after_update: "b2",
})]
#[case::verification_token_organization_id_fkey(FkCase {
    conname: "LiteLLM_VerificationToken_organization_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_VerificationToken" (token, organization_id) VALUES ('tok1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_OrganizationTable" SET organization_id = 'o2' WHERE organization_id = 'o1';"#,
    read_sql: r#"SELECT organization_id FROM "LiteLLM_VerificationToken" WHERE token = 'tok1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o2';"#,
    expected_after_update: "o2",
})]
#[case::end_user_table_budget_id_fkey(FkCase {
    conname: "LiteLLM_EndUserTable_budget_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_EndUserTable" (user_id, budget_id) VALUES ('eu1', 'b1');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'b2' WHERE budget_id = 'b1';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_EndUserTable" WHERE user_id = 'eu1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b2';"#,
    expected_after_update: "b2",
})]
#[case::team_membership_budget_id_fkey(FkCase {
    conname: "LiteLLM_TeamMembership_budget_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, budget_id) VALUES ('u1', 't1', 'b1');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'b2' WHERE budget_id = 'b1';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_TeamMembership" WHERE user_id = 'u1' AND team_id = 't1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b2';"#,
    expected_after_update: "b2",
})]
#[case::organization_membership_user_id_fkey(FkCase {
    conname: "LiteLLM_OrganizationMembership_user_id_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id) VALUES ('u1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_UserTable" SET user_id = 'u2' WHERE user_id = 'u1';"#,
    read_sql: r#"SELECT user_id FROM "LiteLLM_OrganizationMembership" WHERE organization_id = 'o1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = 'u2';"#,
    expected_after_update: "u2",
})]
#[case::organization_membership_organization_id_fkey(FkCase {
    conname: "LiteLLM_OrganizationMembership_organization_id_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'b1', 't', 't');
       INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id) VALUES ('u1', 'o1');"#,
    update_sql: r#"UPDATE "LiteLLM_OrganizationTable" SET organization_id = 'o2' WHERE organization_id = 'o1';"#,
    read_sql: r#"SELECT organization_id FROM "LiteLLM_OrganizationMembership" WHERE user_id = 'u1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_OrganizationTable" WHERE organization_id = 'o2';"#,
    expected_after_update: "o2",
})]
#[case::organization_membership_budget_id_fkey(FkCase {
    conname: "LiteLLM_OrganizationMembership_budget_id_fkey",
    delete_rule: 'n',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('bOrg', 't', 't');
       INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ('o1', 'a', 'bOrg', 't', 't');
       INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('bM', 't', 't');
       INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id, budget_id) VALUES ('u1', 'o1', 'bM');"#,
    update_sql: r#"UPDATE "LiteLLM_BudgetTable" SET budget_id = 'bM2' WHERE budget_id = 'bM';"#,
    read_sql: r#"SELECT budget_id FROM "LiteLLM_OrganizationMembership" WHERE user_id = 'u1' AND organization_id = 'o1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_BudgetTable" WHERE budget_id = 'bM2';"#,
    expected_after_update: "bM2",
})]
#[case::invitation_link_user_id_fkey(FkCase {
    conname: "LiteLLM_InvitationLink_user_id_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u2');
       INSERT INTO "LiteLLM_InvitationLink" (id, user_id, expires_at, created_at, created_by, updated_at, updated_by) VALUES ('i1', 'u1', now(), now(), 'u2', now(), 'u2');"#,
    update_sql: r#"UPDATE "LiteLLM_UserTable" SET user_id = 'u1x' WHERE user_id = 'u1';"#,
    read_sql: r#"SELECT user_id FROM "LiteLLM_InvitationLink" WHERE id = 'i1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = 'u1x';"#,
    expected_after_update: "u1x",
})]
#[case::invitation_link_created_by_fkey(FkCase {
    conname: "LiteLLM_InvitationLink_created_by_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u2');
       INSERT INTO "LiteLLM_InvitationLink" (id, user_id, expires_at, created_at, created_by, updated_at, updated_by) VALUES ('i1', 'u2', now(), now(), 'u1', now(), 'u2');"#,
    update_sql: r#"UPDATE "LiteLLM_UserTable" SET user_id = 'u1x' WHERE user_id = 'u1';"#,
    read_sql: r#"SELECT created_by FROM "LiteLLM_InvitationLink" WHERE id = 'i1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = 'u1x';"#,
    expected_after_update: "u1x",
})]
#[case::invitation_link_updated_by_fkey(FkCase {
    conname: "LiteLLM_InvitationLink_updated_by_fkey",
    delete_rule: 'r',
    setup_sql: r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u1');
       INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('u2');
       INSERT INTO "LiteLLM_InvitationLink" (id, user_id, expires_at, created_at, created_by, updated_at, updated_by) VALUES ('i1', 'u2', now(), now(), 'u2', now(), 'u1');"#,
    update_sql: r#"UPDATE "LiteLLM_UserTable" SET user_id = 'u1x' WHERE user_id = 'u1';"#,
    read_sql: r#"SELECT updated_by FROM "LiteLLM_InvitationLink" WHERE id = 'i1'"#,
    delete_sql: r#"DELETE FROM "LiteLLM_UserTable" WHERE user_id = 'u1x';"#,
    expected_after_update: "u1x",
})]
#[tokio::test]
async fn baseline_foreign_keys(#[case] case: FkCase) {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    client.batch_execute(case.setup_sql).await.unwrap();
    assert_rule(&client, case.conname, case.delete_rule).await;
    client.batch_execute(case.update_sql).await.unwrap();
    let after_update: String = client.query_one(case.read_sql, &[]).await.unwrap().get(0);
    assert_eq!(after_update, case.expected_after_update);
    if case.delete_rule == 'r' {
        let error = client.batch_execute(case.delete_sql).await.unwrap_err();
        assert_eq!(error.code(), Some(&SqlState::FOREIGN_KEY_VIOLATION));
    } else {
        client.batch_execute(case.delete_sql).await.unwrap();
        let after_delete: Option<String> =
            client.query_one(case.read_sql, &[]).await.unwrap().get(0);
        assert_eq!(after_delete, None);
    }
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_reapply() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    client.batch_execute(migration_sql()).await.unwrap();
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_reapply_rows_preserved() {
    let (fixture, client, tasks) = setup().await;
    client.batch_execute(migration_sql()).await.unwrap();
    insert_budget(&client, "b1").await;
    exec(
        &client,
        r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_id, credential_name, credential_values, created_by, updated_by) VALUES ('c1', 'n', '{"api_key":"sk-raw-secret"}', 't', 't')"#,
    )
    .await;
    exec(
        &client,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id, password) VALUES ('u1', 'raw-password')"#,
    )
    .await;
    client.batch_execute(migration_sql()).await.unwrap();
    let budget_count = scalar_i64(
        &client,
        r#"SELECT count(*) FROM "LiteLLM_BudgetTable" WHERE budget_id = 'b1'"#,
    )
    .await;
    assert_eq!(budget_count, 1);
    let credential_values = scalar_text(
        &client,
        r#"SELECT credential_values::TEXT FROM "LiteLLM_CredentialsTable" WHERE credential_id = 'c1'"#,
    )
    .await;
    assert!(credential_values.contains("sk-raw-secret"));
    let password = scalar_text(
        &client,
        r#"SELECT password FROM "LiteLLM_UserTable" WHERE user_id = 'u1'"#,
    )
    .await;
    assert_eq!(password, "raw-password");
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_reapply_drifted_existing_table_not_repaired() {
    let (fixture, client, tasks) = setup().await;
    client
        .batch_execute(
            r#"CREATE TABLE IF NOT EXISTS "LiteLLM_BudgetTable" ("budget_id" TEXT NOT NULL, CONSTRAINT "LiteLLM_BudgetTable_pkey" PRIMARY KEY ("budget_id"));"#,
        )
        .await
        .unwrap();
    client.batch_execute(migration_sql()).await.unwrap();
    let exists: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'LiteLLM_BudgetTable' AND column_name = 'max_budget')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_reapply_foreign_key_name_in_other_schema_suppresses_creation() {
    let (fixture, client, tasks) = setup().await;
    client
        .batch_execute(
            r#"CREATE SCHEMA other_schema;
               CREATE TABLE other_schema.dummy (id INT);
               ALTER TABLE other_schema.dummy ADD CONSTRAINT "LiteLLM_OrganizationTable_budget_id_fkey" CHECK (true);"#,
        )
        .await
        .unwrap();
    client.batch_execute(migration_sql()).await.unwrap();
    let real_fk_count: i64 = client
        .query_one(
            r#"SELECT count(*) FROM pg_constraint WHERE conrelid = '"LiteLLM_OrganizationTable"'::regclass AND contype = 'f'"#,
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(real_fk_count, 0);
    teardown(client, tasks, fixture).await;
}

#[tokio::test]
async fn baseline_reapply_independent_databases_do_not_collide() {
    let (fixture_a, client_a, tasks_a) = setup().await;
    let (fixture_b, client_b, tasks_b) = setup().await;

    client_a.batch_execute(migration_sql()).await.unwrap();
    client_a
        .batch_execute(
            r#"ALTER TABLE "LiteLLM_OrganizationTable" DROP CONSTRAINT "LiteLLM_OrganizationTable_budget_id_fkey";
               CREATE SCHEMA other_schema;
               CREATE TABLE other_schema.dummy (id INT);
               ALTER TABLE other_schema.dummy ADD CONSTRAINT "LiteLLM_OrganizationTable_budget_id_fkey" CHECK (true);"#,
        )
        .await
        .unwrap();
    client_a.batch_execute(migration_sql()).await.unwrap();

    client_b.batch_execute(migration_sql()).await.unwrap();

    let fk_query = r#"SELECT count(*) FROM pg_constraint WHERE conrelid = '"LiteLLM_OrganizationTable"'::regclass AND contype = 'f'"#;
    let fk_in_a: i64 = client_a.query_one(fk_query, &[]).await.unwrap().get(0);
    let fk_in_b: i64 = client_b.query_one(fk_query, &[]).await.unwrap().get(0);
    assert_eq!(fk_in_a, 0);
    assert_eq!(fk_in_b, 1);

    teardown(client_a, tasks_a, fixture_a).await;
    teardown(client_b, tasks_b, fixture_b).await;
}

#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::key::{DeletedVerificationToken, LiteLLMVerificationToken};
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::types::{Json, ToSql};
use tokio_postgres::{NoTls, Row};
use uuid::Uuid;

use crate::audit_events::identity::CallerIdentity;
use crate::key_archive::clock::ArchiveClock;
use crate::key_archive::records::transform_verification_tokens_to_deleted_records;
use crate::key_archive::{KeyArchiveError, persist_deleted_verification_tokens};

use super::parameters::archived_parameters;
use super::save_deleted_verification_token_records;

const DELETED_AT: &str = "2026-05-06T07:08:09.123Z";

const CREATE_TABLE: &str = r#"CREATE TABLE "LiteLLM_DeletedVerificationToken" (
    "id" TEXT NOT NULL,
    "token" TEXT NOT NULL,
    "key_name" TEXT,
    "key_alias" TEXT,
    "soft_budget_cooldown" BOOLEAN NOT NULL DEFAULT false,
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "expires" TIMESTAMP(3),
    "models" TEXT[],
    "aliases" JSONB NOT NULL DEFAULT '{}',
    "config" JSONB NOT NULL DEFAULT '{}',
    "user_id" TEXT,
    "team_id" TEXT,
    "agent_id" TEXT,
    "project_id" TEXT,
    "permissions" JSONB NOT NULL DEFAULT '{}',
    "max_parallel_requests" INTEGER,
    "metadata" JSONB NOT NULL DEFAULT '{}',
    "blocked" BOOLEAN,
    "tpm_limit" BIGINT,
    "rpm_limit" BIGINT,
    "tpd_limit" BIGINT,
    "max_budget" DOUBLE PRECISION,
    "budget_duration" TEXT,
    "budget_reset_at" TIMESTAMP(3),
    "allowed_cache_controls" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "allowed_routes" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "key_type" TEXT,
    "policies" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "access_group_ids" TEXT[] DEFAULT ARRAY[]::TEXT[],
    "model_spend" JSONB NOT NULL DEFAULT '{}',
    "model_max_budget" JSONB NOT NULL DEFAULT '{}',
    "budget_fallbacks" JSONB NOT NULL DEFAULT '{}',
    "router_settings" JSONB DEFAULT '{}',
    "budget_id" TEXT,
    "organization_id" TEXT,
    "object_permission_id" TEXT,
    "created_at" TIMESTAMP(3),
    "created_by" TEXT,
    "updated_at" TIMESTAMP(3),
    "updated_by" TEXT,
    "settings_updated_at" TIMESTAMP(3),
    "last_active" TIMESTAMP(3),
    "rotation_count" INTEGER DEFAULT 0,
    "auto_rotate" BOOLEAN DEFAULT false,
    "rotation_interval" TEXT,
    "last_rotation_at" TIMESTAMP(3),
    "key_rotation_at" TIMESTAMP(3),
    "deleted_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "deleted_by" TEXT,
    "deleted_by_api_key" TEXT,
    "litellm_changed_by" TEXT,
    CONSTRAINT "LiteLLM_DeletedVerificationToken_pkey" PRIMARY KEY ("id")
)"#;

const SELECT_ALL: &str = r#"SELECT * FROM "LiteLLM_DeletedVerificationToken""#;
const SELECT_COUNT: &str = r#"SELECT COUNT(*) FROM "LiteLLM_DeletedVerificationToken""#;
const SELECT_TOKENS: &str =
    r#"SELECT "token", "key_name" FROM "LiteLLM_DeletedVerificationToken" ORDER BY "key_name""#;

const SAMPLE_TOKEN: &str = r#"{
    "token": "sk-hashed-abc123",
    "key_name": "primary-key",
    "key_alias": "alias-one",
    "spend": 12.5,
    "total_spend": 40.25,
    "max_budget": 100.75,
    "expires": "2027-03-04T05:06:07.890Z",
    "models": ["gpt-4o", "claude-3"],
    "aliases": {"team": "alpha"},
    "config": {"region": "eu"},
    "user_id": "user-1",
    "team_id": "team-1",
    "agent_id": "agent-1",
    "project_id": "project-1",
    "max_parallel_requests": 7,
    "metadata": {"tier": "gold"},
    "tpm_limit": 1000,
    "rpm_limit": 500,
    "tpd_limit": 100000,
    "budget_duration": "30d",
    "budget_reset_at": "2027-01-02T00:00:00.500",
    "allowed_cache_controls": ["no-cache"],
    "allowed_routes": ["/chat/completions"],
    "key_type": "default",
    "permissions": {"manage_keys": true},
    "model_spend": {"gpt-4o": 3.25},
    "model_max_budget": {"gpt-4o": 50.5},
    "budget_fallbacks": {"gpt-4o": ["gpt-4o-mini"]},
    "soft_budget_cooldown": true,
    "blocked": false,
    "litellm_budget_table": {"budget_id": "budget-row"},
    "budget_id": "budget-1",
    "org_id": "org-1",
    "created_at": "2026-01-02T03:04:05.678",
    "created_by": "creator-1",
    "updated_at": "2026-02-03T04:05:06.789",
    "updated_by": "editor-1",
    "settings_updated_at": "2026-02-04T05:06:07.891",
    "last_active": "2026-02-05T06:07:08.912",
    "object_permission_id": "perm-1",
    "object_permission": {"object_permission_id": "perm-1"},
    "access_group_ids": ["group-1", "group-2"],
    "rotation_count": 3,
    "auto_rotate": true,
    "rotation_interval": "7d",
    "last_rotation_at": "2026-02-06T07:08:09.123",
    "key_rotation_at": "2026-02-07T08:09:10.234",
    "router_settings": {"num_retries": 2},
    "budget_limits": [{"budget_id": "budget-row"}]
}"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Database {
    fixture: PostgresFixture,
    pool: Pool,
}

impl Database {
    async fn start() -> Self {
        let url = Environment::parse_from(["key-archive-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![CREATE_TABLE.to_owned()])
                .resolve()
                .await
                .expect("a Postgres database fixture");
        let config = fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses");
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(2)
            .build()
            .expect("the fixture pool builds");
        Self { fixture, pool }
    }

    async fn client(&self) -> Client {
        self.pool.get().await.expect("a fixture connection")
    }

    async fn query(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) -> Vec<Row> {
        self.client()
            .await
            .query(statement, parameters)
            .await
            .expect("the fixture query returns rows")
    }

    async fn query_one(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) -> Row {
        self.client()
            .await
            .query_one(statement, parameters)
            .await
            .expect("the fixture query returns a row")
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

struct FixedClock {
    instant: DateTime<Utc>,
}

impl ArchiveClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.instant
    }
}

fn fixed_instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(DELETED_AT)
        .expect("the clock instant parses")
        .with_timezone(&Utc)
}

fn instant(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .expect("the sample timestamp parses")
}

fn sample(json: &str) -> LiteLLMVerificationToken {
    serde_json::from_str(json).expect("the sample token parses")
}

fn sample_caller() -> CallerIdentity {
    CallerIdentity {
        user_id: Some(String::from("deleter-1")),
        api_key: Some(SecretString::from("sk-caller-hash")),
        ..CallerIdentity::default()
    }
}

fn record_from(json: &str) -> DeletedVerificationToken {
    let mut records = transform_verification_tokens_to_deleted_records(
        &[sample(json)],
        &sample_caller(),
        Some("changer-1"),
        &FixedClock {
            instant: fixed_instant(),
        },
    );
    records.pop().expect("one record for one token")
}

async fn only_row(database: &Database) -> Row {
    database.query_one(SELECT_ALL, &[]).await
}

fn assert_identity(row: &Row, record: &DeletedVerificationToken) {
    assert_eq!(
        row.get::<_, String>("id"),
        record.id.clone().expect("the record carries an id")
    );
    let archived = record
        .token
        .token
        .as_ref()
        .map(|secret| secret.expose_secret().as_str())
        .expect("the record carries a token");
    assert_eq!(row.get::<_, String>("token"), archived);
}

fn assert_accounting(row: &Row, record: &DeletedVerificationToken) {
    assert_eq!(
        row.get::<_, bool>("soft_budget_cooldown"),
        record.token.soft_budget_cooldown.0
    );
    assert_eq!(
        row.get::<_, f64>("spend").to_bits(),
        record.token.spend.as_f64().to_bits()
    );
    assert_eq!(
        row.get::<_, f64>("total_spend").to_bits(),
        record.token.total_spend.as_f64().to_bits()
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("expires"),
        Some(instant("2027-03-04T05:06:07.890"))
    );
}

fn assert_texts(row: &Row, record: &DeletedVerificationToken) {
    let expected = [
        ("key_name", &record.token.key_name),
        ("key_alias", &record.token.key_alias),
        ("user_id", &record.token.user_id),
        ("team_id", &record.token.team_id),
        ("agent_id", &record.token.agent_id),
        ("project_id", &record.token.project_id),
        ("budget_duration", &record.token.budget_duration),
        ("key_type", &record.token.key_type),
        ("budget_id", &record.token.budget_id),
        ("object_permission_id", &record.token.object_permission_id),
        ("created_by", &record.token.created_by),
        ("updated_by", &record.token.updated_by),
        ("rotation_interval", &record.token.rotation_interval),
    ];
    expected.iter().for_each(|(column, value)| {
        assert_eq!(row.get::<_, Option<String>>(*column), **value, "{column}");
    });
}

fn assert_json(row: &Row, record: &DeletedVerificationToken) {
    assert_eq!(
        row.get::<_, Json<Payload>>("aliases").0,
        record.token.aliases
    );
    assert_eq!(row.get::<_, Json<Payload>>("config").0, record.token.config);
    assert_eq!(
        row.get::<_, Json<Payload>>("permissions").0,
        record.token.permissions
    );
    assert_eq!(
        row.get::<_, Json<Payload>>("metadata").0,
        record.token.metadata
    );
    assert_eq!(
        row.get::<_, Json<Payload>>("model_spend").0,
        record.token.model_spend
    );
    assert_eq!(
        row.get::<_, Json<Payload>>("model_max_budget").0,
        record.token.model_max_budget
    );
    assert_eq!(
        row.get::<_, Json<BTreeMap<String, Vec<String>>>>("budget_fallbacks")
            .0,
        record.token.budget_fallbacks
    );
    let router = row
        .get::<_, Option<Json<Payload>>>("router_settings")
        .expect("the full record carries router settings");
    assert_eq!(
        router.0,
        record
            .token
            .router_settings
            .clone()
            .expect("the full token carries router settings")
    );
}

fn assert_limits(row: &Row, record: &DeletedVerificationToken) {
    assert_eq!(
        row.get::<_, Option<bool>>("blocked"),
        record.token.blocked.map(|flag| flag.0)
    );
    assert_eq!(
        row.get::<_, Option<i64>>("tpm_limit"),
        record.token.tpm_limit
    );
    assert_eq!(
        row.get::<_, Option<i64>>("rpm_limit"),
        record.token.rpm_limit
    );
    assert_eq!(
        row.get::<_, Option<i64>>("tpd_limit"),
        record.token.tpd_limit
    );
    let max_budget = row
        .get::<_, Option<f64>>("max_budget")
        .expect("the full record carries a max budget");
    let archived = record
        .token
        .max_budget
        .expect("the full token carries a max budget");
    assert_eq!(max_budget.to_bits(), archived.as_f64().to_bits());
}

fn assert_rotation(row: &Row, record: &DeletedVerificationToken) {
    let parallel = row
        .get::<_, Option<i32>>("max_parallel_requests")
        .expect("the full record carries a parallel limit");
    let archived_parallel = record
        .token
        .max_parallel_requests
        .expect("the full token carries a parallel limit");
    assert_eq!(i64::from(parallel), archived_parallel);
    let rotations = row
        .get::<_, Option<i32>>("rotation_count")
        .expect("the full record carries a rotation count");
    let archived_rotations = record
        .token
        .rotation_count
        .0
        .expect("the full token carries a rotation count");
    assert_eq!(i64::from(rotations), archived_rotations);
    assert_eq!(
        row.get::<_, Option<bool>>("auto_rotate"),
        record.token.auto_rotate.0.map(|flag| flag.0)
    );
}

fn assert_arrays(row: &Row, record: &DeletedVerificationToken) {
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("models"),
        Some(vec![String::from("gpt-4o"), String::from("claude-3")])
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("allowed_cache_controls"),
        Some(vec![String::from("no-cache")])
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("allowed_routes"),
        Some(vec![String::from("/chat/completions")])
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("access_group_ids"),
        record.token.access_group_ids
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("policies"),
        Some(Vec::new())
    );
}

fn assert_stamps(row: &Row) {
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("created_at"),
        Some(instant("2026-01-02T03:04:05.678"))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("updated_at"),
        Some(instant("2026-02-03T04:05:06.789"))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("settings_updated_at"),
        Some(instant("2026-02-04T05:06:07.891"))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("last_active"),
        Some(instant("2026-02-05T06:07:08.912"))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("last_rotation_at"),
        Some(instant("2026-02-06T07:08:09.123"))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("key_rotation_at"),
        Some(instant("2026-02-07T08:09:10.234"))
    );
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("budget_reset_at"),
        Some(instant("2027-01-02T00:00:00.500"))
    );
}

fn assert_deletion(row: &Row, record: &DeletedVerificationToken) {
    assert_eq!(
        row.get::<_, Option<String>>("organization_id"),
        record.organization_id
    );
    assert_eq!(
        row.get::<_, NaiveDateTime>("deleted_at"),
        instant("2026-05-06T07:08:09.123")
    );
    assert_eq!(
        row.get::<_, Option<String>>("deleted_by"),
        record.deleted_by
    );
    let actor = record
        .deleted_by_api_key
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    assert_eq!(row.get::<_, Option<String>>("deleted_by_api_key"), actor);
    assert_eq!(
        row.get::<_, Option<String>>("litellm_changed_by"),
        record.litellm_changed_by
    );
}

fn assert_null_texts(row: &Row) {
    let columns = [
        "key_name",
        "key_alias",
        "user_id",
        "team_id",
        "agent_id",
        "project_id",
        "budget_duration",
        "key_type",
        "budget_id",
        "organization_id",
        "object_permission_id",
        "created_by",
        "updated_by",
        "rotation_interval",
        "deleted_by",
        "deleted_by_api_key",
        "litellm_changed_by",
    ];
    columns
        .iter()
        .for_each(|column| assert_eq!(row.get::<_, Option<String>>(*column), None, "{column}"));
}

fn assert_null_scalars(row: &Row) {
    assert_eq!(row.get::<_, Option<NaiveDateTime>>("expires"), None);
    assert_eq!(row.get::<_, Option<NaiveDateTime>>("budget_reset_at"), None);
    assert_eq!(row.get::<_, Option<NaiveDateTime>>("created_at"), None);
    assert_eq!(row.get::<_, Option<NaiveDateTime>>("updated_at"), None);
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("settings_updated_at"),
        None
    );
    assert_eq!(row.get::<_, Option<NaiveDateTime>>("last_active"), None);
    assert_eq!(
        row.get::<_, Option<NaiveDateTime>>("last_rotation_at"),
        None
    );
    assert_eq!(row.get::<_, Option<NaiveDateTime>>("key_rotation_at"), None);
    assert_eq!(row.get::<_, Option<i32>>("max_parallel_requests"), None);
    assert_eq!(row.get::<_, Option<i64>>("tpm_limit"), None);
    assert_eq!(row.get::<_, Option<i64>>("rpm_limit"), None);
    assert_eq!(row.get::<_, Option<i64>>("tpd_limit"), None);
    assert_eq!(row.get::<_, Option<f64>>("max_budget"), None);
    assert_eq!(row.get::<_, Option<bool>>("blocked"), None);
    assert_eq!(row.get::<_, Option<Vec<String>>>("access_group_ids"), None);
    assert_eq!(row.get::<_, Option<Json<Payload>>>("router_settings"), None);
}

fn assert_minimal_defaults(row: &Row) {
    assert!(!row.get::<_, bool>("soft_budget_cooldown"));
    assert_eq!(row.get::<_, f64>("spend").to_bits(), 0.0_f64.to_bits());
    assert_eq!(
        row.get::<_, f64>("total_spend").to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(row.get::<_, Json<Payload>>("aliases").0, Payload::default());
    assert_eq!(row.get::<_, Json<Payload>>("config").0, Payload::default());
    assert_eq!(
        row.get::<_, Json<Payload>>("permissions").0,
        Payload::default()
    );
    assert_eq!(
        row.get::<_, Json<Payload>>("metadata").0,
        Payload::default()
    );
    assert_eq!(
        row.get::<_, Json<Payload>>("model_spend").0,
        Payload::default()
    );
    assert_eq!(
        row.get::<_, Json<Payload>>("model_max_budget").0,
        Payload::default()
    );
    assert_eq!(
        row.get::<_, Json<BTreeMap<String, Vec<String>>>>("budget_fallbacks")
            .0,
        BTreeMap::new()
    );
}

fn assert_minimal_arrays(row: &Row) {
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("models"),
        Some(Vec::new())
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("allowed_cache_controls"),
        Some(Vec::new())
    );
    assert_eq!(
        row.get::<_, Option<Vec<String>>>("allowed_routes"),
        Some(Vec::new())
    );
    assert_eq!(row.get::<_, Option<i32>>("rotation_count"), Some(0));
    assert_eq!(row.get::<_, Option<bool>>("auto_rotate"), Some(false));
}

#[tokio::test]
async fn insert_writes_the_record_postgres() {
    let database = Database::start().await;
    let record = record_from(SAMPLE_TOKEN);
    let client = database.client().await;
    save_deleted_verification_token_records(&client, std::slice::from_ref(&record))
        .await
        .unwrap();
    let row = only_row(&database).await;
    assert_identity(&row, &record);
    assert_accounting(&row, &record);
    assert_texts(&row, &record);
    assert_json(&row, &record);
    assert_limits(&row, &record);
    assert_rotation(&row, &record);
    assert_arrays(&row, &record);
    assert_stamps(&row);
    assert_deletion(&row, &record);
    database.close().await;
}

#[tokio::test]
async fn insert_leaves_absent_values_null_postgres() {
    let database = Database::start().await;
    let mut records = transform_verification_tokens_to_deleted_records(
        &[sample(r#"{"token": "sk-minimal"}"#)],
        &CallerIdentity::default(),
        None,
        &FixedClock {
            instant: fixed_instant(),
        },
    );
    let record = records.pop().expect("one record for one token");
    let client = database.client().await;
    save_deleted_verification_token_records(&client, std::slice::from_ref(&record))
        .await
        .unwrap();
    let row = only_row(&database).await;
    assert_null_texts(&row);
    assert_null_scalars(&row);
    assert_minimal_defaults(&row);
    assert_minimal_arrays(&row);
    assert_eq!(
        row.get::<_, String>("id"),
        record.id.clone().expect("the record carries an id")
    );
    assert_eq!(
        row.get::<_, NaiveDateTime>("deleted_at"),
        instant("2026-05-06T07:08:09.123")
    );
    database.close().await;
}

#[tokio::test]
async fn insert_writes_the_whole_batch_postgres() {
    let database = Database::start().await;
    let first = record_from(r#"{"token": "sk-batch-a", "key_name": "batch-a"}"#);
    let second = record_from(r#"{"token": "sk-batch-b", "key_name": "batch-b"}"#);
    let client = database.client().await;
    save_deleted_verification_token_records(&client, &[first, second])
        .await
        .unwrap();
    let rows = database.query(SELECT_TOKENS, &[]).await;
    let found: Vec<(String, String)> = rows
        .iter()
        .map(|row| {
            (
                row.get::<_, String>("token"),
                row.get::<_, String>("key_name"),
            )
        })
        .collect();
    assert_eq!(
        found,
        vec![
            (String::from("sk-batch-a"), String::from("batch-a")),
            (String::from("sk-batch-b"), String::from("batch-b")),
        ]
    );
    database.close().await;
}

#[tokio::test]
async fn insert_of_no_records_sends_no_statement_postgres() {
    let database = Database::start().await;
    let client = database.client().await;
    save_deleted_verification_token_records(&client, &[])
        .await
        .unwrap();
    let row = database.query_one(SELECT_COUNT, &[]).await;
    assert_eq!(row.get::<_, i64>(0), 0);
    database.close().await;
}

#[tokio::test]
async fn insert_of_a_duplicate_id_reports_a_database_error_postgres() {
    let database = Database::start().await;
    let record = record_from(SAMPLE_TOKEN);
    let client = database.client().await;
    save_deleted_verification_token_records(&client, std::slice::from_ref(&record))
        .await
        .unwrap();
    let error = save_deleted_verification_token_records(&client, std::slice::from_ref(&record))
        .await
        .expect_err("the second write collides on the id");
    assert!(matches!(&error, KeyArchiveError::Database { .. }));
    assert_eq!(error.to_string(), "the deleted key archive write failed");
    assert!(!error.to_string().contains("sk-hashed-abc123"));
    database.close().await;
}

#[tokio::test]
async fn insert_of_a_batch_with_one_duplicate_writes_nothing_postgres() {
    let database = Database::start().await;
    let record = record_from(SAMPLE_TOKEN);
    let fresh = record_from(r#"{"token": "sk-fresh", "key_name": "fresh-key"}"#);
    let client = database.client().await;
    save_deleted_verification_token_records(&client, std::slice::from_ref(&record))
        .await
        .unwrap();
    let error = save_deleted_verification_token_records(&client, &[fresh, record])
        .await
        .expect_err("the batch collides on the id");
    assert!(matches!(&error, KeyArchiveError::Database { .. }));
    let row = database.query_one(SELECT_COUNT, &[]).await;
    assert_eq!(row.get::<_, i64>(0), 1);
    database.close().await;
}

#[tokio::test]
async fn insert_runs_inside_the_callers_transaction_postgres() {
    let database = Database::start().await;
    let record = record_from(SAMPLE_TOKEN);
    let mut client = database.client().await;
    let rolled_back = client.transaction().await.expect("a transaction");
    save_deleted_verification_token_records(&rolled_back, std::slice::from_ref(&record))
        .await
        .unwrap();
    let inside: i64 = rolled_back
        .query_one(SELECT_COUNT, &[])
        .await
        .expect("a count inside the rolled back transaction")
        .get(0);
    assert_eq!(inside, 1);
    rolled_back.rollback().await.expect("the rollback");
    let after: i64 = client
        .query_one(SELECT_COUNT, &[])
        .await
        .expect("a count after the rollback")
        .get(0);
    assert_eq!(after, 0);
    let committed = client.transaction().await.expect("a transaction");
    save_deleted_verification_token_records(&committed, std::slice::from_ref(&record))
        .await
        .unwrap();
    committed.commit().await.expect("the commit");
    let kept: i64 = client
        .query_one(SELECT_COUNT, &[])
        .await
        .expect("a count after the commit")
        .get(0);
    assert_eq!(kept, 1);
    database.close().await;
}

#[tokio::test]
async fn persist_transforms_and_saves_postgres() {
    let database = Database::start().await;
    let client = database.client().await;
    persist_deleted_verification_tokens(
        &client,
        &[sample(SAMPLE_TOKEN)],
        &sample_caller(),
        Some("changer-1"),
        &FixedClock {
            instant: fixed_instant(),
        },
    )
    .await
    .unwrap();
    let row = only_row(&database).await;
    assert_eq!(row.get::<_, String>("token"), "sk-hashed-abc123");
    assert_eq!(
        row.get::<_, Option<String>>("organization_id"),
        Some(String::from("org-1"))
    );
    assert_eq!(
        row.get::<_, Option<String>>("deleted_by"),
        Some(String::from("deleter-1"))
    );
    assert_eq!(
        row.get::<_, NaiveDateTime>("deleted_at"),
        instant("2026-05-06T07:08:09.123")
    );
    let id = row.get::<_, String>("id");
    let parsed = Uuid::parse_str(&id).expect("the archived id is a uuid");
    assert_eq!(parsed.get_version_num(), 4);
    database.close().await;
}

#[test]
fn non_text_models_are_refused() {
    let record = record_from(r#"{"token": "sk-x", "models": [1]}"#);
    let Err(error) = archived_parameters(std::slice::from_ref(&record)) else {
        panic!("a numeric model name is refused");
    };
    assert!(matches!(
        error,
        KeyArchiveError::NonTextValue { column: "models" }
    ));
}

#[test]
fn non_text_cache_controls_are_refused() {
    let record = record_from(r#"{"token": "sk-x", "allowed_cache_controls": [1]}"#);
    let Err(error) = archived_parameters(std::slice::from_ref(&record)) else {
        panic!("a numeric cache control is refused");
    };
    assert!(matches!(
        error,
        KeyArchiveError::NonTextValue {
            column: "allowed_cache_controls"
        }
    ));
}

#[test]
fn non_text_allowed_routes_are_refused() {
    let record = record_from(r#"{"token": "sk-x", "allowed_routes": ["/chat", true]}"#);
    let Err(error) = archived_parameters(std::slice::from_ref(&record)) else {
        panic!("a boolean route is refused");
    };
    assert!(matches!(
        error,
        KeyArchiveError::NonTextValue {
            column: "allowed_routes"
        }
    ));
}

#[test]
fn unparseable_expires_is_refused() {
    let record = record_from(r#"{"token": "sk-x", "expires": "tomorrow"}"#);
    let Err(error) = archived_parameters(std::slice::from_ref(&record)) else {
        panic!("a text expiry that is not a timestamp is refused");
    };
    assert!(matches!(
        error,
        KeyArchiveError::InvalidTimestamp { column: "expires" }
    ));
}

#![cfg(test)]

use super::decode::{MAX_FIELD_BYTES, TokenDecodeError, decode_token_record, to_model};
use super::record::DbRecord;
use super::table::columns::TokenColumn;
use super::table::{
    RawTokenRow, TOKEN_TABLE_ATTRIBUTE, TokenFilter, TokenTableError, TokenValue, TokenWrite,
};
use super::{TokenRepositoryError, VerificationTokenRepository};
use crate::budget_permission_rows::JsonData;
use crate::database_refresh::handle::{PoolFuture, PoolTransport};
use crate::database_replica::availability::Availability;
use crate::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use crate::database_replica::pinned::{WriterPinnedClient, pin_writer};
use crate::database_replica::routing::RoutingDatabase;
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::key_hash::KeyHash;
use crate::postgres_store::errors::DbError;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

fn data(value: &str) -> JsonData {
    serde_json::from_str(value).unwrap()
}

fn text(value: &str) -> JsonData {
    JsonData::String(String::from(value))
}

fn map(fields: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    let mut mapping = BTreeMap::new();
    for (field, value) in fields {
        mapping.insert(String::from(*field), value.clone());
    }
    mapping
}

fn record_of(fields: &[(&str, JsonData)]) -> DbRecord {
    DbRecord {
        model_dump: None,
        dict: None,
        mapping: Some(map(fields)),
        pairs: None,
    }
}

fn convert(fields: &[(&str, JsonData)]) -> LiteLLMVerificationToken {
    to_model(Some(&record_of(fields))).unwrap().unwrap()
}

fn error_of(fields: &[(&str, JsonData)]) -> TokenDecodeError {
    to_model(Some(&record_of(fields))).unwrap_err()
}

#[test]
fn none_record_stays_none() {
    assert!(to_model(None).unwrap().is_none());
}

fn assert_nine(key: &LiteLLMVerificationToken) {
    assert_eq!(key.aliases.0.get("a"), Some(&data("1")));
    assert_eq!(key.config.0.get("b"), Some(&data("2")));
    assert_eq!(key.permissions.0.get("c"), Some(&data("3")));
    assert_eq!(key.metadata.0.get("d"), Some(&data("4")));
    assert_eq!(key.model_spend.0.get("e"), Some(&data("5")));
    assert_eq!(key.model_max_budget.0.get("f"), Some(&data("6")));
    assert_eq!(
        key.router_settings.as_ref().unwrap().0.get("g"),
        Some(&data("7"))
    );
    assert_eq!(
        key.budget_limits
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .0
            .get("h"),
        Some(&data("8"))
    );
    assert_eq!(
        key.litellm_budget_table
            .as_ref()
            .unwrap()
            .0
            .get("budget_id"),
        Some(&data(r#""b""#))
    );
}

#[test]
fn each_nine_json_fields_decodes_once() {
    let key = convert(&[
        ("aliases", text(r#"{"a":1}"#)),
        ("config", text(r#"{"b":2}"#)),
        ("permissions", text(r#"{"c":3}"#)),
        ("metadata", text(r#"{"d":4}"#)),
        ("model_spend", text(r#"{"e":5}"#)),
        ("model_max_budget", text(r#"{"f":6}"#)),
        ("router_settings", text(r#"{"g":7}"#)),
        ("budget_limits", text(r#"[{"h":8}]"#)),
        ("litellm_budget_table", text(r#"{"budget_id":"b"}"#)),
    ]);
    assert_nine(&key);
}

#[test]
fn already_decoded_values_unchanged() {
    let fields = [
        ("aliases", data(r#"{"a":1}"#)),
        ("config", data(r#"{"b":2}"#)),
        ("permissions", data(r#"{"c":3}"#)),
        ("metadata", data(r#"{"d":4}"#)),
        ("model_spend", data(r#"{"e":5}"#)),
        ("model_max_budget", data(r#"{"f":6}"#)),
        ("router_settings", data(r#"{"g":7}"#)),
        ("budget_limits", data(r#"[{"h":8}]"#)),
        ("litellm_budget_table", data(r#"{"budget_id":"b"}"#)),
    ];
    let key = convert(&fields);
    assert_nine(&key);
}

#[test]
fn malformed_json_propagates() {
    let error = error_of(&[("aliases", text("{not json"))]);
    assert!(matches!(
        error,
        TokenDecodeError::FieldJson { field, .. } if field == "aliases"
    ));
    let truncated = error_of(&[("metadata", text(r#"{"a":1"#))]);
    assert!(matches!(
        truncated,
        TokenDecodeError::FieldJson { field, .. } if field == "metadata"
    ));
}

#[test]
fn scalar_and_array_values_reach_row_validation() {
    assert!(matches!(
        error_of(&[("metadata", text("3"))]),
        TokenDecodeError::Model { .. }
    ));
    assert!(matches!(
        error_of(&[("budget_limits", text("3"))]),
        TokenDecodeError::Model { .. }
    ));
    let empty = convert(&[("budget_limits", text("[]"))]);
    assert_eq!(empty.budget_limits, Some(Vec::new()));
    let listed = convert(&[("budget_limits", text(r#"[{"max_budget":3}]"#))]);
    assert_eq!(
        listed
            .budget_limits
            .unwrap()
            .first()
            .unwrap()
            .0
            .get("max_budget"),
        Some(&data("3"))
    );
}

#[test]
fn nullable_json_null_vs_required_dict() {
    let key = convert(&[
        ("router_settings", text("null")),
        ("litellm_budget_table", text("null")),
        ("budget_limits", text("null")),
    ]);
    assert!(key.router_settings.is_none());
    assert!(key.litellm_budget_table.is_none());
    assert!(key.budget_limits.is_none());
    assert!(matches!(
        error_of(&[("aliases", text("null"))]),
        TokenDecodeError::Model { .. }
    ));
    assert!(matches!(
        error_of(&[("metadata", text("null"))]),
        TokenDecodeError::Model { .. }
    ));
}

#[rstest]
#[case::budget_fallbacks("budget_fallbacks", r#"{"a":["b"]}"#)]
#[case::models("models", r#"["m"]"#)]
#[case::allowed_routes("allowed_routes", r#"["r"]"#)]
#[case::object_permission("object_permission", r#"{"object_permission_id":"p"}"#)]
fn unlisted_fields_are_not_json_decoded(#[case] field: &str, #[case] value: &str) {
    let error = error_of(&[(field, text(value))]);
    assert!(
        matches!(error, TokenDecodeError::Model { .. }),
        "{field} was decoded: {error:?}"
    );
}

#[test]
fn double_encoded_text_not_recursively_decoded() {
    let key = convert(&[("aliases", text(r#"{"a":"{\"b\":1}"}"#))]);
    assert_eq!(
        key.aliases.0.get("a"),
        Some(&JsonData::String(String::from(r#"{"b":1}"#)))
    );
    assert!(matches!(
        error_of(&[("metadata", text(r#""{\"b\":1}""#))]),
        TokenDecodeError::Model { .. }
    ));
}

#[test]
fn missing_and_null_org_id_copy_organization_id() {
    let missing = convert(&[("organization_id", data(r#""org-a""#))]);
    assert_eq!(missing.org_id.as_deref(), Some("org-a"));
    let null = convert(&[
        ("org_id", JsonData::Null),
        ("organization_id", data(r#""org-b""#)),
    ]);
    assert_eq!(null.org_id.as_deref(), Some("org-b"));
}

#[test]
fn explicit_org_id_including_empty_wins() {
    let empty = convert(&[
        ("org_id", data(r#""""#)),
        ("organization_id", data(r#""org-a""#)),
    ]);
    assert_eq!(empty.org_id.as_deref(), Some(""));
    let kept = convert(&[
        ("org_id", data(r#""kept""#)),
        ("organization_id", data(r#""org-a""#)),
    ]);
    assert_eq!(kept.org_id.as_deref(), Some("kept"));
}

#[test]
fn null_organization_id_not_copied() {
    let null = convert(&[("organization_id", JsonData::Null)]);
    assert!(null.org_id.is_none());
    let missing = convert(&[("token", data(r#""k""#))]);
    assert!(missing.org_id.is_none());
}

#[test]
fn empty_organization_id_copied() {
    let key = convert(&[("organization_id", data(r#""""#))]);
    assert_eq!(key.org_id.as_deref(), Some(""));
}

#[test]
fn conversion_does_not_mutate_input() {
    let record = record_of(&[
        ("aliases", text(r#"{"a":1}"#)),
        ("organization_id", data(r#""org""#)),
    ]);
    let before = record.clone();
    let key = to_model(Some(&record)).unwrap().unwrap();
    assert_eq!(key.aliases.0.get("a"), Some(&data("1")));
    assert_eq!(record, before);
}

#[test]
fn record_projection_uses_base_precedence() {
    let dump = DbRecord {
        model_dump: Some(map(&[("source", data(r#""model_dump""#))])),
        dict: Some(map(&[("source", data(r#""dict""#))])),
        mapping: Some(map(&[("source", data(r#""mapping""#))])),
        pairs: Some(vec![(String::from("source"), data(r#""pairs""#))]),
    };
    assert_eq!(dump.project().get("source"), Some(&data(r#""model_dump""#)));
    let direct = DbRecord {
        model_dump: None,
        dict: Some(map(&[("source", data(r#""dict""#))])),
        mapping: Some(map(&[("source", data(r#""mapping""#))])),
        pairs: None,
    };
    assert_eq!(direct.project().get("source"), Some(&data(r#""dict""#)));
    let mapping = DbRecord {
        model_dump: None,
        dict: None,
        mapping: Some(map(&[("source", data(r#""mapping""#))])),
        pairs: Some(vec![(String::from("source"), data(r#""pairs""#))]),
    };
    assert_eq!(mapping.project().get("source"), Some(&data(r#""mapping""#)));
    let pairs = DbRecord {
        model_dump: None,
        dict: None,
        mapping: None,
        pairs: Some(vec![
            (String::from("source"), data("1")),
            (String::from("source"), data("2")),
        ]),
    };
    assert_eq!(pairs.project().get("source"), Some(&data("2")));
}

#[test]
fn raw_token_row_debug_redacts_the_stored_identifier() {
    let row = RawTokenRow(JsonData::Object(map(&[(
        "token",
        data(r#""legacy-literal""#),
    )])));
    assert_eq!(format!("{row:?}"), "RawTokenRow([REDACTED])");
}

#[test]
fn key_hash_preserves_prehashed_identifier() {
    let prehashed = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let key = KeyHash::from(prehashed);
    assert_eq!(key.expose_secret(), prehashed);
    assert_eq!(key, KeyHash::from(prehashed));
    assert_ne!(
        key,
        KeyHash::from("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdee")
    );
    assert_eq!(format!("{key:?}"), "[REDACTED]");
}

#[test]
fn key_hash_preserves_sql_punctuation_literal() {
    let literal = r#"key'); DROP TABLE "LiteLLM_VerificationToken"; --"#;
    let key = KeyHash::from(literal);
    assert_eq!(key.expose_secret(), literal);
}

#[test]
fn native_token_record_field_limits() {
    let bounded = format!(r#"{{"k":"{}"}}"#, "a".repeat(MAX_FIELD_BYTES - 8));
    assert_eq!(bounded.len(), MAX_FIELD_BYTES);
    let key = convert(&[("aliases", text(&bounded))]);
    assert!(key.aliases.0.contains_key("k"));
    let over = format!(r#"{{"k":"{}"}}"#, "a".repeat(MAX_FIELD_BYTES - 7));
    assert!(matches!(
        error_of(&[("aliases", text(&over))]),
        TokenDecodeError::FieldTooLarge { .. }
    ));
    let deep = format!(r#"{{"k":{}1{}}}"#, "[".repeat(63), "]".repeat(63));
    let nested = convert(&[("config", text(&deep))]);
    assert!(nested.config.0.contains_key("k"));
    let too_deep = format!(r#"{{"k":{}1{}}}"#, "[".repeat(64), "]".repeat(64));
    assert!(matches!(
        error_of(&[("config", text(&too_deep))]),
        TokenDecodeError::FieldTooDeep { .. }
    ));
}

#[rstest]
fn replays_the_token_record_corpus(#[files("fuzz/corpus/token_record/*")] path: PathBuf) {
    drop(decode_token_record(&fs::read(path).unwrap()));
}

#[test]
fn quoted_braces_stay_outside_the_depth_count() {
    let escaped = format!("{{\"k\":\"\\\"{}\"}}", "{".repeat(64));
    let key = convert(&[("config", text(&escaped))]);
    assert!(key.config.0.contains_key("k"));
    let nested = format!("{}{}", "[".repeat(63), "]".repeat(63));
    let repeated = format!("[{nested},{nested}]");
    assert!(matches!(
        error_of(&[("budget_limits", text(&repeated))]),
        TokenDecodeError::Model { .. }
    ));
}

const ADDED_COLUMNS: &str = r#"
ALTER TABLE "LiteLLM_VerificationToken"
    ADD COLUMN IF NOT EXISTS total_spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN IF NOT EXISTS agent_id TEXT,
    ADD COLUMN IF NOT EXISTS project_id TEXT,
    ADD COLUMN IF NOT EXISTS allowed_routes TEXT[] DEFAULT ARRAY[]::TEXT[],
    ADD COLUMN IF NOT EXISTS key_type TEXT,
    ADD COLUMN IF NOT EXISTS policies TEXT[] DEFAULT ARRAY[]::TEXT[],
    ADD COLUMN IF NOT EXISTS access_group_ids TEXT[],
    ADD COLUMN IF NOT EXISTS budget_fallbacks JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS object_permission_id TEXT,
    ADD COLUMN IF NOT EXISTS settings_updated_at TIMESTAMP(3),
    ADD COLUMN IF NOT EXISTS last_active TIMESTAMP(3),
    ADD COLUMN IF NOT EXISTS tpd_limit BIGINT,
    ADD COLUMN IF NOT EXISTS rotation_count INTEGER,
    ADD COLUMN IF NOT EXISTS auto_rotate BOOLEAN,
    ADD COLUMN IF NOT EXISTS rotation_interval TEXT,
    ADD COLUMN IF NOT EXISTS last_rotation_at TIMESTAMP(3),
    ADD COLUMN IF NOT EXISTS key_rotation_at TIMESTAMP(3),
    ADD COLUMN IF NOT EXISTS router_settings JSONB,
    ADD COLUMN IF NOT EXISTS budget_limits JSONB;
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct TestClient {
    pool: Pool,
}

impl DatabaseLifecycle for TestClient {
    type Error = Infallible;

    fn is_connected(&self) -> bool {
        true
    }

    fn connect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn disconnect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn recreate<'database>(
        &'database self,
        _url: &'database SecretString,
        _transport: Option<&'database PoolTransport>,
        _generation: Option<u64>,
    ) -> PoolFuture<'database, bool, Infallible> {
        Box::pin(async { Ok(true) })
    }

    fn start_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn stop_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }
}

impl DatabaseClient for TestClient {
    type Attribute = Pool;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<Pool>>, Infallible> {
        Ok((name == TOKEN_TABLE_ATTRIBUTE).then(|| Attribute::Model(self.pool.clone())))
    }

    fn token_refresh_enabled(&self) -> bool {
        false
    }

    fn token_label(&self) -> &'static str {
        "database token"
    }

    fn minted_url(&self) -> Result<Option<SecretString>, Infallible> {
        Ok(None)
    }
}

fn fixture_pool(url: &SecretString) -> Pool {
    let config = url.expose_secret().parse().unwrap();
    Pool::builder(Manager::new(config, NoTls)).build().unwrap()
}

async fn database() -> (PostgresFixture, Pool) {
    let configured = Environment::parse_from(["key-repository-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(configured.as_ref())
        .resolve()
        .await
        .unwrap();
    let pool = fixture_pool(&fixture.options.url);
    let client = pool.get().await.unwrap();
    client
        .batch_execute(include_str!(
            "../../../../tools/postgres-fixture/fixtures/baseline.sql"
        ))
        .await
        .unwrap();
    client.batch_execute(ADDED_COLUMNS).await.unwrap();
    drop(client);
    (fixture, pool)
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

fn direct(client: &TestClient) -> VerificationTokenRepository<'_, TestClient> {
    VerificationTokenRepository {
        database: Some(WriterPinnedClient::Writer(client)),
    }
}

async fn insert(pool: &Pool, sql: &str, parameters: &[&(dyn ToSql + Sync)]) {
    let client = pool.get().await.unwrap();
    client.execute(sql, parameters).await.unwrap();
}

async fn seed(pool: &Pool, token: &str) {
    let sql =
        r#"INSERT INTO "LiteLLM_VerificationToken" (token, models) VALUES ($1, ARRAY[]::TEXT[])"#;
    insert(pool, sql, &[&token]).await;
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

async fn seed_legacy(pool: &Pool, token: &str) {
    let aliases = json_string(r#"{"a":1}"#);
    let config = json_string(r#"{"b":2}"#);
    let permissions = json_string(r#"{"c":3}"#);
    let metadata = json_string(r#"{"d":4}"#);
    let model_spend = json_string(r#"{"e":5}"#);
    let model_max_budget = json_string(r#"{"f":6}"#);
    let router_settings = json_string(r#"{"g":7}"#);
    let budget_limits = json_string(r#"[{"h":8}]"#);
    let sql = r#"INSERT INTO "LiteLLM_VerificationToken" (token, aliases, config, permissions, metadata, model_spend, model_max_budget, router_settings, budget_limits, models) VALUES ($1, $2::text::jsonb, $3::text::jsonb, $4::text::jsonb, $5::text::jsonb, $6::text::jsonb, $7::text::jsonb, $8::text::jsonb, $9::text::jsonb, ARRAY[]::TEXT[])"#;
    let parameters: [&(dyn ToSql + Sync); 9] = [
        &token,
        &aliases,
        &config,
        &permissions,
        &metadata,
        &model_spend,
        &model_max_budget,
        &router_settings,
        &budget_limits,
    ];
    insert(pool, sql, &parameters).await;
}

async fn seed_invalid_aliases(pool: &Pool, token: &str) {
    let aliases = json_string("{not json");
    let sql = r#"INSERT INTO "LiteLLM_VerificationToken" (token, aliases, models) VALUES ($1, $2::text::jsonb, ARRAY[]::TEXT[])"#;
    insert(pool, sql, &[&token, &aliases]).await;
}

fn key(value: &str) -> KeyHash {
    KeyHash::from(value)
}

fn token_filter(value: &str) -> TokenFilter {
    TokenFilter(vec![(TokenColumn::Token, TokenValue::KeyHash(key(value)))])
}

async fn assert_raw_legacy(repository: &VerificationTokenRepository<'_, TestClient>) {
    let raw = repository
        .table()
        .unwrap()
        .find_unique(&key("legacy"))
        .await
        .unwrap()
        .unwrap();
    let JsonData::Object(raw_fields) = &raw.0 else {
        panic!("the raw row is not an object");
    };
    assert_eq!(
        raw_fields.get("aliases"),
        Some(&JsonData::String(String::from(r#"{"a":1}"#)))
    );
    assert_eq!(
        raw_fields.get("router_settings"),
        Some(&JsonData::String(String::from(r#"{"g":7}"#)))
    );
}

async fn assert_converted_legacy(repository: &VerificationTokenRepository<'_, TestClient>) {
    let converted = repository.find_many_in(vec![key("legacy")]).await.unwrap();
    let stored = converted.first().unwrap();
    assert_eq!(stored.aliases.0.get("a"), Some(&data("1")));
    assert_eq!(stored.config.0.get("b"), Some(&data("2")));
    assert_eq!(stored.permissions.0.get("c"), Some(&data("3")));
    assert_eq!(stored.metadata.0.get("d"), Some(&data("4")));
    assert_eq!(stored.model_spend.0.get("e"), Some(&data("5")));
    assert_eq!(stored.model_max_budget.0.get("f"), Some(&data("6")));
    assert_eq!(
        stored.router_settings.as_ref().unwrap().0.get("g"),
        Some(&data("7"))
    );
    assert_eq!(
        stored
            .budget_limits
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .0
            .get("h"),
        Some(&data("8"))
    );
}

#[tokio::test]
async fn postgres_raw_table_and_converted_rows_differ() {
    let (fixture, pool) = database().await;
    seed_legacy(&pool, "legacy").await;
    let client = TestClient { pool: pool.clone() };
    let repository = direct(&client);
    assert_raw_legacy(&repository).await;
    assert_converted_legacy(&repository).await;
    close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_bound_identifier_is_not_rehashed() {
    let (fixture, pool) = database().await;
    let literal = r#"legacy'); DROP TABLE "LiteLLM_VerificationToken"; --"#;
    seed(&pool, "prehashed-custom-identifier").await;
    seed(&pool, literal).await;
    let client = TestClient { pool: pool.clone() };
    let repository = direct(&client);
    let table = repository.table().unwrap();
    assert!(
        table
            .find_unique(&key("prehashed-custom-identifier"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(table.find_unique(&key(literal)).await.unwrap().is_some());
    let hex = "9f2c9b0f8e4d7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b";
    assert!(table.find_unique(&key(hex)).await.unwrap().is_none());
    let rows: i64 = pool
        .get()
        .await
        .unwrap()
        .query_one(r#"SELECT count(*) FROM "LiteLLM_VerificationToken""#, &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 2);
    close(fixture, pool).await;
}

fn routing_over(writer_pool: &Pool, reader_pool: &Pool) -> RoutingDatabase<TestClient> {
    RoutingDatabase {
        writer: TestClient {
            pool: writer_pool.clone(),
        },
        reader: TestClient {
            pool: reader_pool.clone(),
        },
        availability: Availability::default(),
        reader_url: None,
    }
}

async fn assert_routed_reads(routing: &RoutingDatabase<TestClient>) {
    let repository = VerificationTokenRepository {
        database: Some(WriterPinnedClient::Routed(routing)),
    };
    let table = repository.table().unwrap();
    assert!(
        table
            .find_unique(&key("reader-key"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        table
            .find_unique(&key("writer-key"))
            .await
            .unwrap()
            .is_none()
    );
    let write = TokenWrite(BTreeMap::from([(
        TokenColumn::KeyAlias,
        TokenValue::Text(String::from("renamed")),
    )]));
    assert_eq!(
        table
            .update(&token_filter("writer-key"), &write)
            .await
            .unwrap()
            .len(),
        1
    );
}

async fn assert_writer_row(writer_pool: &Pool) {
    let writer_client = writer_pool.get().await.unwrap();
    let alias: String = writer_client
        .query_one(
            r#"SELECT key_alias FROM "LiteLLM_VerificationToken" WHERE token = $1"#,
            &[&"writer-key"],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(alias, "renamed");
}

async fn assert_writer_pin(routing: &RoutingDatabase<TestClient>) {
    let pinned = pin_writer(WriterPinnedClient::Routed(routing));
    let repository = VerificationTokenRepository {
        database: Some(pinned),
    };
    let table = repository.table().unwrap();
    assert!(
        table
            .find_unique(&key("writer-key"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        table
            .find_unique(&key("reader-key"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn supplied_db_routing_and_writer_pinning() {
    let (writer_fixture, writer_pool) = database().await;
    let (reader_fixture, reader_pool) = database().await;
    seed(&writer_pool, "writer-key").await;
    seed(&reader_pool, "reader-key").await;
    let routing = routing_over(&writer_pool, &reader_pool);
    assert_routed_reads(&routing).await;
    assert_writer_row(&writer_pool).await;
    assert_writer_pin(&routing).await;
    close(writer_fixture, writer_pool).await;
    close(reader_fixture, reader_pool).await;
}

#[tokio::test]
async fn missing_client_error_before_query() {
    let repository = VerificationTokenRepository::<TestClient> { database: None };
    let error = repository.find_many_in(Vec::new()).await.err().unwrap();
    assert_eq!(
        error.to_string(),
        "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
    assert!(matches!(
        error,
        TokenRepositoryError::Table(TokenTableError::Database(DbError::NoDbConnected))
    ));
    assert!(repository.table().is_err());
}

#[tokio::test]
async fn empty_duplicate_and_large_membership_lookup() {
    let (fixture, pool) = database().await;
    seed(&pool, "one").await;
    seed(&pool, "two").await;
    let client = TestClient { pool: pool.clone() };
    let repository = direct(&client);
    assert!(
        repository
            .find_many_in(Vec::new())
            .await
            .unwrap()
            .is_empty()
    );
    let duplicated = repository
        .find_many_in(vec![key("one"), key("one"), key("two")])
        .await
        .unwrap();
    assert_eq!(duplicated.len(), 2);
    let mut many: Vec<KeyHash> = (0_u16..5000)
        .map(|index| KeyHash::from(format!("absent-{index}")))
        .collect();
    many.push(key("one"));
    let found = repository.find_many_in(many).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        found
            .first()
            .unwrap()
            .token
            .as_ref()
            .unwrap()
            .expose_secret(),
        "one"
    );
    close(fixture, pool).await;
}

async fn assert_invalid_record_error(repository: &VerificationTokenRepository<'_, TestClient>) {
    let error = repository
        .find_many_in(vec![key("broken")])
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        TokenRepositoryError::Table(TokenTableError::Database(DbError::InvalidRecord {
            source: TokenDecodeError::FieldJson { .. }
        }))
    ));
}

async fn assert_sql_error_leaves_row(
    repository: &VerificationTokenRepository<'_, TestClient>,
    pool: &Pool,
) {
    let table = repository.table().unwrap();
    assert!(table.find_unique(&key("broken")).await.unwrap().is_some());
    let write = TokenWrite(BTreeMap::from([(
        TokenColumn::Aliases,
        TokenValue::Text(String::from("not json")),
    )]));
    let update_error = table
        .update(&token_filter("broken"), &write)
        .await
        .unwrap_err();
    assert!(matches!(
        update_error,
        TokenTableError::Database(DbError::SqlState { .. })
    ));
    let stored: String = pool
        .get()
        .await
        .unwrap()
        .query_one(
            r#"SELECT aliases::text FROM "LiteLLM_VerificationToken" WHERE token = $1"#,
            &[&"broken"],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, r#""{not json""#);
}

#[tokio::test]
async fn database_errors_propagate_and_rows_unchanged() {
    let (fixture, pool) = database().await;
    seed_invalid_aliases(&pool, "broken").await;
    let client = TestClient { pool: pool.clone() };
    let repository = direct(&client);
    assert_invalid_record_error(&repository).await;
    assert_sql_error_leaves_row(&repository, &pool).await;
    close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_find_many_filters_and_limits() {
    let (fixture, pool) = database().await;
    seed(&pool, "one").await;
    seed(&pool, "two").await;
    seed(&pool, "three").await;
    let client = TestClient { pool: pool.clone() };
    let repository = direct(&client);
    let table = repository.table().unwrap();
    let filter = TokenFilter(vec![(
        TokenColumn::Token,
        TokenValue::Text(String::from("one")),
    )]);
    let filtered = table.find_many(&filter, Some(1)).await.unwrap();
    assert_eq!(filtered.len(), 1);
    let unfiltered = table
        .find_many(&TokenFilter(Vec::new()), None)
        .await
        .unwrap();
    assert_eq!(unfiltered.len(), 3);
    close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_update_and_delete_respect_the_filter() {
    let (fixture, pool) = database().await;
    seed(&pool, "target").await;
    seed(&pool, "keep").await;
    let client = TestClient { pool: pool.clone() };
    let repository = direct(&client);
    let table = repository.table().unwrap();
    let write = TokenWrite(BTreeMap::from([(
        TokenColumn::KeyAlias,
        TokenValue::Text(String::from("changed")),
    )]));
    table.update(&token_filter("target"), &write).await.unwrap();
    let kept = table.find_unique(&key("keep")).await.unwrap().unwrap();
    let JsonData::Object(kept_fields) = &kept.0 else {
        panic!("the kept row is not an object");
    };
    assert_eq!(kept_fields.get("key_alias"), Some(&JsonData::Null));
    table.delete(&token_filter("target")).await.unwrap();
    assert!(table.find_unique(&key("target")).await.unwrap().is_none());
    assert!(table.find_unique(&key("keep")).await.unwrap().is_some());
    close(fixture, pool).await;
}

fn typed_create() -> TokenWrite {
    TokenWrite(BTreeMap::from([
        (
            TokenColumn::Token,
            TokenValue::KeyHash(key(
                r#"typed'); DROP TABLE "LiteLLM_VerificationToken"; --"#,
            )),
        ),
        (
            TokenColumn::KeyAlias,
            TokenValue::Text(String::from("aliased")),
        ),
        (
            TokenColumn::Spend,
            TokenValue::Number(rust_decimal::Decimal::new(125, 2)),
        ),
        (TokenColumn::Blocked, TokenValue::Bool(true)),
        (
            TokenColumn::Models,
            TokenValue::TextArray(vec![String::from("m1"), String::from("m2")]),
        ),
        (TokenColumn::Aliases, TokenValue::Json(data(r#"{"a":1}"#))),
        (TokenColumn::BudgetId, TokenValue::Null),
    ]))
}

#[tokio::test]
async fn postgres_typed_create_and_update_bind_values() {
    let (fixture, pool) = database().await;
    let client = TestClient { pool: pool.clone() };
    let repository = direct(&client);
    let table = repository.table().unwrap();
    let literal = r#"typed'); DROP TABLE "LiteLLM_VerificationToken"; --"#;
    let created = table.create(&typed_create()).await.unwrap();
    let JsonData::Object(created_fields) = &created.0 else {
        panic!("the created row is not an object");
    };
    assert_eq!(
        created_fields.get("key_alias"),
        Some(&JsonData::String(String::from("aliased")))
    );
    assert_eq!(created_fields.get("spend"), Some(&data("1.25")));
    assert_eq!(created_fields.get("blocked"), Some(&JsonData::Bool(true)));
    assert_eq!(created_fields.get("models"), Some(&data(r#"["m1","m2"]"#)));
    assert_eq!(created_fields.get("aliases"), Some(&data(r#"{"a":1}"#)));
    assert_eq!(created_fields.get("budget_id"), Some(&JsonData::Null));
    let update = TokenWrite(BTreeMap::from([(
        TokenColumn::KeyAlias,
        TokenValue::Text(String::from("changed")),
    )]));
    let updated = table.update(&token_filter(literal), &update).await.unwrap();
    assert_eq!(updated.len(), 1);
    let JsonData::Object(updated_fields) = &updated.first().unwrap().0 else {
        panic!("the updated row is not an object");
    };
    assert_eq!(
        updated_fields.get("key_alias"),
        Some(&JsonData::String(String::from("changed")))
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_typed_upsert_and_delete_bind_values() {
    let (fixture, pool) = database().await;
    let client = TestClient { pool: pool.clone() };
    let repository = direct(&client);
    let table = repository.table().unwrap();
    let literal = r#"typed'); DROP TABLE "LiteLLM_VerificationToken"; --"#;
    let update = TokenWrite(BTreeMap::from([(
        TokenColumn::KeyAlias,
        TokenValue::Text(String::from("first")),
    )]));
    let inserted = table.upsert(&typed_create(), &update).await.unwrap();
    let JsonData::Object(inserted_fields) = &inserted.0 else {
        panic!("the inserted row is not an object");
    };
    assert_eq!(
        inserted_fields.get("key_alias"),
        Some(&JsonData::String(String::from("aliased")))
    );
    let replacement = TokenWrite(BTreeMap::from([(
        TokenColumn::KeyAlias,
        TokenValue::Text(String::from("second")),
    )]));
    let conflict = table.upsert(&typed_create(), &replacement).await.unwrap();
    let JsonData::Object(conflict_fields) = &conflict.0 else {
        panic!("the conflicting row is not an object");
    };
    assert_eq!(
        conflict_fields.get("key_alias"),
        Some(&JsonData::String(String::from("second")))
    );
    assert_eq!(table.delete(&token_filter(literal)).await.unwrap().len(), 1);
    assert!(table.find_unique(&key(literal)).await.unwrap().is_none());
    close(fixture, pool).await;
}

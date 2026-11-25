#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::sso_assertion_rows::MemoryAssertionRows;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use super::super::Rotation;
use super::super::error::RotationError;
use super::super::postgres::{MODEL_TRANSACTION_TIMEOUT, PostgresRotationStore, StoreError};
use super::support::{
    FakeMcpStore, NEW_KEY, OLD_KEY, RecordingSink, SinkError, cipher, new_key, open, sealed,
    secrets,
};
use crate::config_override_support::environment::RuntimeEnvironment;
use crate::db_row_decrypt::warnings::Warnings;

// The three tables _rotate_master_key writes, with LiteLLM's nullable and NOT NULL columns.
const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ProxyModelTable" (
    model_id TEXT PRIMARY KEY,
    model_name TEXT NOT NULL,
    litellm_params JSONB NOT NULL,
    model_info JSONB,
    blocked BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    created_by TEXT NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_by TEXT NOT NULL
);
CREATE TABLE "LiteLLM_Config" (
    param_name TEXT PRIMARY KEY,
    param_value JSONB
);
CREATE TABLE "LiteLLM_CredentialsTable" (
    credential_name TEXT PRIMARY KEY,
    credential_values JSONB NOT NULL,
    credential_info JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    created_by TEXT NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_by TEXT NOT NULL
);
"#;

// The second model row's UPDATE raises, so the transaction has to roll the first row back.
const FAIL_SECOND_MODEL_UPDATE: &str = r#"
CREATE SEQUENCE model_update_calls;
CREATE FUNCTION fail_second_model_update() RETURNS trigger AS $$
BEGIN
    IF nextval('model_update_calls') = 2 THEN
        RAISE EXCEPTION 'the second model update fails';
    END IF;
    RETURN NEW;
END $$ LANGUAGE plpgsql;
CREATE TRIGGER fail_second_model BEFORE UPDATE ON "LiteLLM_ProxyModelTable"
    FOR EACH ROW EXECUTE FUNCTION fail_second_model_update();
"#;

const FAIL_CONFIG_WRITE: &str = r#"
CREATE FUNCTION fail_config_update() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'the config write fails';
END $$ LANGUAGE plpgsql;
CREATE TRIGGER fail_config BEFORE UPDATE ON "LiteLLM_Config"
    FOR EACH ROW EXECUTE FUNCTION fail_config_update();
"#;

// to_jsonb($2::text) is LiteLLM's create form (the JSON text inside a string); ($2::text)::jsonb is the object form a rotation writes.
const INSERT_MODEL_SQL: &str = r#"INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, model_info, created_by, updated_by, updated_at) VALUES ($1, 'gpt-4o', to_jsonb($2::text), ($3::text)::jsonb, 'creator', 'editor', TIMESTAMP '2000-01-01 00:00:00')"#;
const INSERT_OBJECT_MODEL_SQL: &str = r#"INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, model_info, created_by, updated_by, updated_at) VALUES ($1, 'gpt-4o', ($2::text)::jsonb, ($3::text)::jsonb, 'creator', 'editor', TIMESTAMP '2000-01-01 00:00:00')"#;
const INSERT_CREDENTIAL_SQL: &str = r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_name, credential_values, credential_info, created_by, updated_by) VALUES ($1, ($2::text)::jsonb, '{}'::jsonb, 'creator', 'editor')"#;
const INSERT_NULL_INFO_CREDENTIAL_SQL: &str = r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_name, credential_values, credential_info, created_by, updated_by) VALUES ($1, ($2::text)::jsonb, NULL, 'creator', 'editor')"#;
const INSERT_UNDECODABLE_CREDENTIAL_SQL: &str = r#"INSERT INTO "LiteLLM_CredentialsTable" (credential_name, credential_values, created_by, updated_by) VALUES ($1, to_jsonb($2::text), 'creator', 'editor')"#;
const INSERT_ENVIRONMENT_SQL: &str = r#"INSERT INTO "LiteLLM_Config" (param_name, param_value) VALUES ('environment_variables', to_jsonb($1::text))"#;

const MODEL_ROW_SQL: &str = r#"SELECT model_name, model_info::text, created_by, updated_by, updated_at::text, litellm_params #>> '{}' FROM "LiteLLM_ProxyModelTable" WHERE model_id = $1"#;
const CREDENTIAL_VALUES_SQL: &str = r#"SELECT credential_values #>> '{}' FROM "LiteLLM_CredentialsTable" WHERE credential_name = $1"#;
const CREDENTIAL_TYPE_SQL: &str = r#"SELECT jsonb_typeof(credential_values) FROM "LiteLLM_CredentialsTable" WHERE credential_name = $1"#;
const CREDENTIAL_INFO_TYPE_SQL: &str = r#"SELECT jsonb_typeof(credential_info) FROM "LiteLLM_CredentialsTable" WHERE credential_name = $1"#;
const CREDENTIAL_UPDATED_BY_SQL: &str =
    r#"SELECT updated_by FROM "LiteLLM_CredentialsTable" WHERE credential_name = $1"#;
const ENVIRONMENT_SQL: &str = r#"SELECT param_value #>> '{}' FROM "LiteLLM_Config" WHERE param_name = 'environment_variables'"#;
const ENVIRONMENT_TYPE_SQL: &str = r#"SELECT jsonb_typeof(param_value) FROM "LiteLLM_Config" WHERE param_name = 'environment_variables'"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn object_json(fields: Vec<(&str, String)>) -> String {
    let entries: Vec<String> = fields
        .into_iter()
        .map(|(name, value)| format!(r#""{name}":"{value}""#))
        .collect();
    format!("{{{}}}", entries.join(","))
}

struct StoredModel {
    model_name: String,
    model_info: String,
    created_by: String,
    updated_by: String,
    updated_at: String,
    params: BTreeMap<String, JsonData>,
}

fn assert_sealed(params: &BTreeMap<String, JsonData>, key: &str, plaintext: &str) {
    let JsonData::String(stored) = &params[key] else {
        panic!("the {key} parameter is text");
    };
    assert_eq!(open(stored, NEW_KEY).as_deref(), Some(plaintext));
    assert_eq!(open(stored, OLD_KEY), None);
}

fn parse_object(text: &str) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = serde_json::from_str(text).expect("the stored value is JSON")
    else {
        panic!("the stored value is a JSON object");
    };
    fields
}

struct Harness {
    fixture: PostgresFixture,
    pool: Pool,
    rotation_store: PostgresRotationStore,
    sink: RecordingSink,
    mcp: FakeMcpStore,
    assertions: MemoryAssertionRows,
    cipher: DbCipher,
    key: SecretString,
    warnings: Warnings,
    secrets: SecretLookup,
    runtime: RuntimeEnvironment,
}

fn fixture_pool(fixture: &PostgresFixture) -> Pool {
    let manager = Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses"),
        NoTls,
    );
    Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("the fixture pool builds")
}

impl Harness {
    async fn start(model_timeout: Duration) -> Self {
        let url = Environment::parse_from(["credential-rotation-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
                .resolve()
                .await
                .expect("a Postgres database fixture");
        let pool = fixture_pool(&fixture);
        let store = PostgresStore {
            pool: Some(pool.clone()),
        };
        let mut rotation_store = PostgresRotationStore::new(store, Availability::default());
        rotation_store.model_timeout = model_timeout;
        Self {
            fixture,
            pool,
            rotation_store,
            sink: RecordingSink {
                log: Arc::new(Mutex::new(Vec::new())),
                failed: false,
            },
            mcp: FakeMcpStore::default(),
            assertions: MemoryAssertionRows::default(),
            cipher: cipher(OLD_KEY),
            key: new_key(),
            warnings: Warnings::default(),
            secrets: secrets(),
            runtime: RuntimeEnvironment::default(),
        }
    }

    fn rotation(
        &mut self,
    ) -> Rotation<'_, PostgresRotationStore, FakeMcpStore, MemoryAssertionRows, RecordingSink> {
        Rotation {
            store: &self.rotation_store,
            invalidate: &self.sink,
            mcp: &self.mcp,
            assertions: &self.assertions,
            cipher: &self.cipher,
            new_master_key: &self.key,
            secrets: &self.secrets,
            warnings: &self.warnings,
            runtime: &mut self.runtime,
        }
    }

    async fn execute(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(statement, parameters)
            .await
            .expect("the statement runs");
    }

    async fn batch(&self, statement: &str) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .batch_execute(statement)
            .await
            .expect("the statements run");
    }

    async fn scalar(&self, statement: &str, key: &str) -> Option<String> {
        let client = self.pool.get().await.expect("a fixture connection");
        let row = client
            .query_opt(statement, &[&key])
            .await
            .expect("the query runs");
        row.map(|found| found.get(0))
    }

    async fn seed_model(&self, model_id: &str, params: &str, info: &str) {
        self.execute(INSERT_MODEL_SQL, &[&model_id, &params, &info])
            .await;
    }

    async fn seed_object_model(&self, model_id: &str, params: &str, info: &str) {
        self.execute(INSERT_OBJECT_MODEL_SQL, &[&model_id, &params, &info])
            .await;
    }

    async fn seed_credential(&self, name: &str, values: &str) {
        self.execute(INSERT_CREDENTIAL_SQL, &[&name, &values]).await;
    }

    async fn seed_undecodable_credential(&self, name: &str, values: &str) {
        self.execute(INSERT_UNDECODABLE_CREDENTIAL_SQL, &[&name, &values])
            .await;
    }

    async fn seed_environment(&self, mapping: &str) {
        self.execute(INSERT_ENVIRONMENT_SQL, &[&mapping]).await;
    }

    async fn model(&self, model_id: &str) -> StoredModel {
        let client = self.pool.get().await.expect("a fixture connection");
        let row = client
            .query_one(MODEL_ROW_SQL, &[&model_id])
            .await
            .expect("the model row reads");
        let params: String = row.get(5);
        StoredModel {
            model_name: row.get(0),
            model_info: row.get(1),
            created_by: row.get(2),
            updated_by: row.get(3),
            updated_at: row.get(4),
            params: parse_object(&params),
        }
    }

    async fn credential_values(&self, name: &str) -> BTreeMap<String, JsonData> {
        let values = self
            .scalar(CREDENTIAL_VALUES_SQL, name)
            .await
            .expect("the credential row exists");
        parse_object(&values)
    }

    async fn credential_type(&self, name: &str) -> String {
        self.scalar(CREDENTIAL_TYPE_SQL, name)
            .await
            .expect("the credential row exists")
    }

    async fn credential_updated_by(&self, name: &str) -> String {
        self.scalar(CREDENTIAL_UPDATED_BY_SQL, name)
            .await
            .expect("the credential row exists")
    }

    async fn credential_info_type(&self, name: &str) -> String {
        self.scalar(CREDENTIAL_INFO_TYPE_SQL, name)
            .await
            .expect("the credential row exists")
    }

    async fn environment(&self) -> Option<BTreeMap<String, JsonData>> {
        let client = self.pool.get().await.expect("a fixture connection");
        let row = client
            .query_opt(ENVIRONMENT_SQL, &[])
            .await
            .expect("the query runs");
        row.map(|found| parse_object(&found.get::<usize, String>(0)))
    }

    async fn environment_type(&self) -> String {
        let client = self.pool.get().await.expect("a fixture connection");
        let row = client
            .query_one(ENVIRONMENT_TYPE_SQL, &[])
            .await
            .expect("the query runs");
        row.get(0)
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

// prisma_client.db.tx(timeout=timedelta(minutes=2)): the rotation holds the model transaction for two minutes.
#[test]
fn the_model_transaction_gets_two_minutes() {
    assert_eq!(MODEL_TRANSACTION_TIMEOUT, Duration::from_secs(120));
    let store = PostgresRotationStore::new(PostgresStore { pool: None }, Availability::default());
    assert_eq!(store.model_timeout, MODEL_TRANSACTION_TIMEOUT);
}

// The update writes litellm_params only: both stored forms re-encrypt, and name, model_info and the audit columns stay.
#[tokio::test]
async fn the_model_rotation_rewrites_litellm_params_and_keeps_the_rest() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    let text_params = object_json(vec![
        ("model", sealed("openai/gpt-4o")),
        ("api_base", sealed("https://example.test/v1")),
    ]);
    harness
        .seed_model("m1", &text_params, r#"{"owned_by":"team-a"}"#)
        .await;
    harness
        .seed_object_model(
            "m2",
            &object_json(vec![("model", sealed("openai/gpt-4o-mini"))]),
            r#"{"owned_by":"team-b"}"#,
        )
        .await;
    let before = harness.model("m1").await;
    harness.rotation().run(Some("rotator")).await.unwrap();
    let after = harness.model("m1").await;
    assert_eq!(after.model_name, "gpt-4o");
    assert_eq!(after.model_info, before.model_info);
    assert_eq!(after.created_by, "creator");
    assert_eq!(after.updated_by, "editor");
    assert_ne!(after.updated_at, before.updated_at);
    assert_sealed(&after.params, "model", "openai/gpt-4o");
    assert_sealed(&after.params, "api_base", "https://example.test/v1");
    assert_sealed(
        &harness.model("m2").await.params,
        "model",
        "openai/gpt-4o-mini",
    );
    harness.close().await;
}

#[tokio::test]
async fn a_malformed_model_row_is_skipped() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    harness
        .seed_model(
            "bad",
            &object_json(vec![("api_base", sealed("https://example.test/v1"))]),
            "{}",
        )
        .await;
    harness
        .seed_model(
            "good",
            &object_json(vec![("model", sealed("openai/gpt-4o"))]),
            "{}",
        )
        .await;
    harness.rotation().run(None).await.unwrap();
    let bad = harness.model("bad").await;
    let JsonData::String(api_base) = &bad.params["api_base"] else {
        panic!("the api_base parameter is text");
    };
    assert_eq!(
        open(api_base, OLD_KEY).as_deref(),
        Some("https://example.test/v1")
    );
    assert_sealed(
        &harness.model("good").await.params,
        "model",
        "openai/gpt-4o",
    );
    harness.close().await;
}

#[tokio::test]
async fn a_failing_statement_rolls_back_every_model_row() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    harness.batch(FAIL_SECOND_MODEL_UPDATE).await;
    harness
        .seed_model(
            "m1",
            &object_json(vec![("model", sealed("openai/gpt-4o"))]),
            "{}",
        )
        .await;
    harness
        .seed_model(
            "m2",
            &object_json(vec![("model", sealed("openai/gpt-4o-mini"))]),
            "{}",
        )
        .await;
    let error = harness.rotation().run(None).await.unwrap_err();
    assert!(matches!(
        error,
        RotationError::Store(StoreError::Database(_))
    ));
    let first = harness.model("m1").await;
    let JsonData::String(model) = &first.params["model"] else {
        panic!("the model parameter is text");
    };
    assert_eq!(open(model, OLD_KEY).as_deref(), Some("openai/gpt-4o"));
    assert_eq!(open(model, NEW_KEY), None);
    harness.close().await;
}

#[tokio::test]
async fn a_failing_invalidation_keeps_the_committed_rows() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    harness.sink.failed = true;
    harness
        .seed_model(
            "m1",
            &object_json(vec![("model", sealed("openai/gpt-4o"))]),
            "{}",
        )
        .await;
    let error = harness.rotation().run(None).await.unwrap_err();
    assert!(matches!(error, RotationError::Invalidation(SinkError)));
    assert_sealed(&harness.model("m1").await.params, "model", "openai/gpt-4o");
    harness.close().await;
}

#[tokio::test]
async fn an_expired_model_transaction_writes_nothing() {
    let mut harness = Harness::start(Duration::ZERO).await;
    harness
        .seed_model(
            "m1",
            &object_json(vec![("model", sealed("openai/gpt-4o"))]),
            "{}",
        )
        .await;
    let error = harness.rotation().run(None).await.unwrap_err();
    assert!(matches!(
        error,
        RotationError::Store(StoreError::ModelTimeout(_))
    ));
    let stored = harness.model("m1").await;
    let JsonData::String(model) = &stored.params["model"] else {
        panic!("the model parameter is text");
    };
    assert_eq!(open(model, OLD_KEY).as_deref(), Some("openai/gpt-4o"));
    harness.close().await;
}

#[tokio::test]
async fn an_undecodable_credential_between_two_valid_ones_is_skipped() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    harness
        .seed_credential("c1", &object_json(vec![("api_key", sealed("sk-1"))]))
        .await;
    harness
        .seed_undecodable_credential("bad", r#"{"api_key":"abc"}"#)
        .await;
    harness
        .seed_credential("c2", &object_json(vec![("api_key", sealed("sk-2"))]))
        .await;
    harness.rotation().run(Some("rotator")).await.unwrap();
    assert_sealed(&harness.credential_values("c1").await, "api_key", "sk-1");
    assert_sealed(&harness.credential_values("c2").await, "api_key", "sk-2");
    assert_eq!(harness.credential_type("bad").await, "string");
    harness.close().await;
}

// credential_info is nullable in LiteLLM's schema; the row still rotates and the info column takes the empty payload.
#[tokio::test]
async fn a_credential_row_with_a_null_info_still_rotates() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    let values = object_json(vec![("api_key", sealed("sk-1"))]);
    harness
        .execute(INSERT_NULL_INFO_CREDENTIAL_SQL, &[&"c1", &values])
        .await;
    harness.rotation().run(Some("rotator")).await.unwrap();
    assert_sealed(&harness.credential_values("c1").await, "api_key", "sk-1");
    assert_eq!(harness.credential_info_type("c1").await, "object");
    assert_eq!(harness.credential_updated_by("c1").await, "rotator");
    harness.close().await;
}

// updated_by is NOT NULL, so the null actor fails every credential update and the per-row catch skips them.
#[tokio::test]
async fn a_null_actor_leaves_every_credential_row_unchanged() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    harness
        .seed_credential("c1", &object_json(vec![("api_key", sealed("sk-1"))]))
        .await;
    harness.rotation().run(None).await.unwrap();
    let values = harness.credential_values("c1").await;
    let JsonData::String(stored) = &values["api_key"] else {
        panic!("the api key is text");
    };
    assert_eq!(open(stored, OLD_KEY).as_deref(), Some("sk-1"));
    assert_eq!(open(stored, NEW_KEY), None);
    assert_eq!(harness.credential_updated_by("c1").await, "editor");
    harness.close().await;
}

#[tokio::test]
async fn the_environment_rotation_stores_the_new_mapping_without_the_stdio_flag() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    let mapping = object_json(vec![
        ("OPENAI_API_KEY", sealed("sk-live")),
        ("LITELLM_ENABLE_MCP_STDIO", sealed("true")),
    ]);
    harness.seed_environment(&mapping).await;
    harness.rotation().run(Some("rotator")).await.unwrap();
    let stored = harness.environment().await.expect("the row is there");
    assert_sealed(&stored, "OPENAI_API_KEY", "sk-live");
    assert!(!stored.contains_key("LITELLM_ENABLE_MCP_STDIO"));
    assert_eq!(harness.environment_type().await, "object");
    assert_eq!(
        harness
            .runtime
            .snapshot()
            .values
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("sk-live")
    );
    harness.close().await;
}

#[tokio::test]
async fn a_failed_environment_write_keeps_the_runtime_values() {
    let mut harness = Harness::start(MODEL_TRANSACTION_TIMEOUT).await;
    harness.batch(FAIL_CONFIG_WRITE).await;
    harness
        .seed_environment(&object_json(vec![("OPENAI_API_KEY", sealed("sk-live"))]))
        .await;
    let error = harness.rotation().run(None).await.unwrap_err();
    assert!(matches!(error, RotationError::Store(StoreError::Table(_))));
    assert_eq!(
        harness
            .runtime
            .snapshot()
            .values
            .get("OPENAI_API_KEY")
            .map(String::as_str),
        Some("sk-live")
    );
    let stored = harness.environment().await.expect("the row is there");
    let JsonData::String(value) = &stored["OPENAI_API_KEY"] else {
        panic!("the stored value is text");
    };
    assert_eq!(open(value, OLD_KEY).as_deref(), Some("sk-live"));
    harness.close().await;
}

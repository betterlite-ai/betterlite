#![cfg(test)]

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::encryption_scan::report::MigrationReport;
use betterlite_db::encryption_scan::{DbCipher, MigrationJson};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secrets::secret_codec::Algorithm;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use super::super::callback::CallbackVars;
use super::super::error::MigrationError;
use super::super::postgres::{MigrationStoreError, PostgresMigrationStore};
use super::super::service::{Migration, check_encryption};
use super::support::{
    EXPECTED_LOCATIONS, FakeRotation, RotationCall, RotationError, SALT_KEY,
    callback_settings_metadata, callback_text, callback_vars_metadata, cipher, field_text, json,
    legacy, location_names, location_of, marked, object, open, secret, v2,
};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ProxyModelTable" (
    model_id TEXT PRIMARY KEY,
    model_name TEXT NOT NULL,
    litellm_params JSONB NOT NULL,
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP NOT NULL DEFAULT now()
);
CREATE TABLE "LiteLLM_CredentialsTable" (
    credential_id TEXT PRIMARY KEY,
    credential_name TEXT NOT NULL,
    credential_values JSONB NOT NULL,
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL
);
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    credentials JSONB,
    env_vars JSONB,
    static_headers JSONB,
    env JSONB
);
CREATE TABLE "LiteLLM_MCPUserCredentials" (
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    credential_b64 TEXT NOT NULL,
    PRIMARY KEY (user_id, server_id)
);
CREATE TABLE "LiteLLM_MCPUserEnvVars" (
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    values_b64 TEXT NOT NULL,
    PRIMARY KEY (user_id, server_id)
);
CREATE TABLE "LiteLLM_Config" (
    param_name TEXT PRIMARY KEY,
    param_value JSONB
);
CREATE TABLE "LiteLLM_SSOConfig" (
    id TEXT PRIMARY KEY DEFAULT 'sso_config',
    sso_settings JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP NOT NULL DEFAULT now()
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    metadata JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP NOT NULL DEFAULT now()
);
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    metadata JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP NOT NULL DEFAULT now()
);
"#;

const FAIL_CLOUDZERO_WRITE: &str = r#"
CREATE FUNCTION fail_cloudzero_update() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'the cloudzero write fails';
END $$ LANGUAGE plpgsql;
CREATE TRIGGER fail_cloudzero BEFORE UPDATE ON "LiteLLM_Config"
    FOR EACH ROW WHEN (OLD.param_name = 'cloudzero_settings')
    EXECUTE FUNCTION fail_cloudzero_update();
"#;

const SEED_TIME: &str = "2000-01-01 00:00:00";

const INSERT_MODEL: &str = "INSERT INTO \"LiteLLM_ProxyModelTable\" \
     (model_id, model_name, litellm_params, created_by, updated_by) \
     VALUES ('model-1', 'gpt-4o', ($1::text)::jsonb, 'creator', 'editor')";
const INSERT_CREDENTIAL: &str = "INSERT INTO \"LiteLLM_CredentialsTable\" \
     (credential_id, credential_name, credential_values, created_by, updated_by) \
     VALUES ('cred-1', 'cred', ($1::text)::jsonb, 'creator', 'editor')";
const INSERT_CONFIG_OBJECT: &str =
    "INSERT INTO \"LiteLLM_Config\" (param_name, param_value) VALUES ($1, ($2::text)::jsonb)";
// to_jsonb($2::text) is the string-in-a-Json-column shape Prisma writes for json.dumps(settings).
const INSERT_CONFIG_STRING: &str =
    "INSERT INTO \"LiteLLM_Config\" (param_name, param_value) VALUES ($1, to_jsonb($2::text))";
const INSERT_SSO: &str = "INSERT INTO \"LiteLLM_SSOConfig\" (id, sso_settings, updated_at) \
     VALUES ('sso_config', ($1::text)::jsonb, TIMESTAMP '2000-01-01 00:00:00')";
const INSERT_TEAM_OBJECT: &str = "INSERT INTO \"LiteLLM_TeamTable\" (team_id, metadata, updated_at) \
     VALUES ($1, ($2::text)::jsonb, TIMESTAMP '2000-01-01 00:00:00')";
const INSERT_TEAM_STRING: &str = "INSERT INTO \"LiteLLM_TeamTable\" (team_id, metadata, updated_at) \
     VALUES ($1, to_jsonb($2::text), TIMESTAMP '2000-01-01 00:00:00')";
const INSERT_TOKEN: &str = "INSERT INTO \"LiteLLM_VerificationToken\" (token, metadata, updated_at) \
     VALUES ($1, ($2::text)::jsonb, TIMESTAMP '2000-01-01 00:00:00')";

#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Harness {
    _fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
    availability: Availability,
}

impl Harness {
    async fn start(statements: Vec<String>) -> Self {
        let url = TestEnvironment::parse_from(["credential-migration-tests"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
            .resolve()
            .await
            .expect("a Postgres database fixture");
        let manager = Manager::new(
            fixture
                .options
                .url
                .expose_secret()
                .parse()
                .expect("the fixture URL parses"),
            NoTls,
        );
        let pool = Pool::builder(manager)
            .max_size(4)
            .build()
            .expect("the fixture pool builds");
        let store = PostgresStore {
            pool: Some(pool.clone()),
        };
        Self {
            _fixture: fixture,
            pool,
            store,
            availability: Availability::default(),
        }
    }

    fn migration_store(&self) -> PostgresMigrationStore<'_> {
        PostgresMigrationStore {
            client: PrismaClient {
                writer: &self.store,
                reader: &self.store,
                availability: &self.availability,
            },
        }
    }

    async fn execute(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(statement, parameters)
            .await
            .expect("the statement runs");
    }

    async fn probe(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) -> String {
        let client = self.pool.get().await.expect("a fixture connection");
        let row = client
            .query_one(statement, parameters)
            .await
            .expect("the probe reads");
        row.get::<_, String>(0)
    }

    async fn json_kind(&self, table: &str, column: &str, key_column: &str, key: &str) -> String {
        let sql = format!(
            "SELECT jsonb_typeof(\"{column}\")::text FROM \"{table}\" WHERE \"{key_column}\" = $1"
        );
        self.probe(&sql, &[&key]).await
    }

    async fn json_content(
        &self,
        table: &str,
        column: &str,
        key_column: &str,
        key: &str,
    ) -> MigrationJson {
        let sql =
            format!("SELECT \"{column}\" #>> '{{}}' FROM \"{table}\" WHERE \"{key_column}\" = $1");
        let text = self.probe(&sql, &[&key]).await;
        serde_json::from_str(&text).expect("the stored value is JSON")
    }

    async fn updated_at(&self, table: &str, key_column: &str, key: &str) -> String {
        let sql = format!("SELECT updated_at::text FROM \"{table}\" WHERE \"{key_column}\" = $1");
        self.probe(&sql, &[&key]).await
    }

    async fn snapshot(&self) -> Vec<String> {
        let tables = [
            "LiteLLM_ProxyModelTable",
            "LiteLLM_CredentialsTable",
            "LiteLLM_MCPServerTable",
            "LiteLLM_MCPUserCredentials",
            "LiteLLM_MCPUserEnvVars",
            "LiteLLM_Config",
            "LiteLLM_SSOConfig",
            "LiteLLM_TeamTable",
            "LiteLLM_VerificationToken",
        ];
        let client = self.pool.get().await.expect("a fixture connection");
        let mut values = Vec::new();
        for table in tables {
            let sql = format!(
                "SELECT row_to_json(rows)::text FROM \"{table}\" rows ORDER BY row_to_json(rows)::text"
            );
            let rows = client.query(&sql, &[]).await.expect("the snapshot reads");
            values.extend(rows.iter().map(|row| row.get::<_, String>(0)));
        }
        values
    }
}

async fn seed(harness: &Harness) {
    seed_covered_rows(harness).await;
    seed_config_rows(harness).await;
    seed_sso_and_token_rows(harness).await;
    seed_team_rows(harness).await;
}

async fn seed_covered_rows(harness: &Harness) {
    let model = json(&object(vec![(
        "api_key",
        secret(&legacy(SALT_KEY, "sk-model")),
    )]));
    harness.execute(INSERT_MODEL, &[&model]).await;
    let credential = json(&object(vec![(
        "token",
        secret(&legacy(SALT_KEY, "sk-credential")),
    )]));
    harness.execute(INSERT_CREDENTIAL, &[&credential]).await;
    let environment = json(&object(vec![(
        "DB_PASSWORD",
        secret(&legacy(SALT_KEY, "db-password")),
    )]));
    harness
        .execute(
            INSERT_CONFIG_OBJECT,
            &[&"environment_variables", &environment],
        )
        .await;
}

async fn seed_config_rows(harness: &Harness) {
    let vantage = json(&object(vec![
        ("api_key", secret(&legacy(SALT_KEY, "sk-vantage"))),
        ("integration_token", secret(&v2(SALT_KEY, "token-vantage"))),
    ]));
    harness
        .execute(INSERT_CONFIG_OBJECT, &[&"vantage_settings", &vantage])
        .await;
    let cloudzero = json(&object(vec![(
        "api_key",
        secret(&legacy(SALT_KEY, "sk-cloudzero")),
    )]));
    harness
        .execute(INSERT_CONFIG_STRING, &[&"cloudzero_settings", &cloudzero])
        .await;
}

async fn seed_sso_and_token_rows(harness: &Harness) {
    let sso = json(&object(vec![
        ("client_secret", secret(&legacy(SALT_KEY, "sso-secret"))),
        ("client_id", secret("plain-client")),
    ]));
    harness.execute(INSERT_SSO, &[&sso]).await;
    let token = json(&object(vec![("other", secret("value"))]));
    harness
        .execute(INSERT_TOKEN, &[&"token-clean", &token])
        .await;
}

async fn seed_team_rows(harness: &Harness) {
    let team_marked = json(&callback_vars_metadata(vec![(
        "api_key",
        marked(&legacy(SALT_KEY, "sk-team")),
    )]));
    harness
        .execute(INSERT_TEAM_OBJECT, &[&"team-marked", &team_marked])
        .await;
    let team_clean = json(&callback_settings_metadata(vec![(
        "api_key",
        marked(&v2(SALT_KEY, "sk-done")),
    )]));
    harness
        .execute(INSERT_TEAM_OBJECT, &[&"team-clean", &team_clean])
        .await;
    let team_string = json(&callback_vars_metadata(vec![(
        "api_key",
        marked(&legacy(SALT_KEY, "sk-string")),
    )]));
    harness
        .execute(INSERT_TEAM_STRING, &[&"team-string", &team_string])
        .await;
}

async fn real_run(
    harness: &Harness,
    rotation: &mut FakeRotation,
    cipher: &DbCipher,
    actor: Option<&str>,
) -> Result<MigrationReport, MigrationError<MigrationStoreError, RotationError>> {
    let store = harness.migration_store();
    let mut service = Migration {
        store: &store,
        rotation,
        cipher,
        transform: CallbackVars,
        actor,
        dry_run: false,
    };
    service.run().await
}

fn assert_attested(report: &MigrationReport) {
    assert_eq!(location_names(report), EXPECTED_LOCATIONS);
    assert_eq!(location_of(report, "model_table").legacy, 1);
    assert_eq!(location_of(report, "credentials").legacy, 1);
    assert_eq!(location_of(report, "mcp_server").scanned, 0);
    assert_eq!(
        location_of(report, "config_environment_variables").legacy,
        1
    );
    let team = location_of(report, "team.callback_vars");
    assert_eq!((team.scanned, team.legacy, team.already_v2), (2, 1, 1));
    assert_eq!(
        location_of(report, "verification_token.callback_vars").scanned,
        0
    );
    let vantage = location_of(report, "vantage_settings");
    assert_eq!(
        (vantage.scanned, vantage.legacy, vantage.already_v2),
        (2, 1, 1)
    );
    assert_eq!(location_of(report, "cloudzero_settings").legacy, 1);
    let sso = location_of(report, "sso_config");
    assert_eq!((sso.scanned, sso.legacy, sso.plaintext), (2, 1, 1));
    assert_eq!(
        report
            .as_dict()
            .expect("the report projects")
            .residual_legacy,
        7
    );
}

#[tokio::test]
async fn attestation_reads_every_location_with_the_gate_off() {
    let harness = Harness::start(vec![SCHEMA.to_owned()]).await;
    seed(&harness).await;
    let before = harness.snapshot().await;
    let cipher = cipher(Algorithm::XSalsa20Poly1305, Some(SALT_KEY));
    let store = harness.migration_store();
    let report = check_encryption(&store, &cipher)
        .await
        .expect("the attestation runs with the gate off");
    assert_attested(&report);
    assert_eq!(harness.snapshot().await, before);
}

#[tokio::test]
async fn real_run_writes_json_strings_and_reports_exact_counts() {
    let harness = Harness::start(vec![SCHEMA.to_owned()]).await;
    seed(&harness).await;
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let report = real_run(&harness, &mut rotation, &cipher, Some("admin"))
        .await
        .expect("the migration writes every location");
    assert_report_counts(&report, &rotation);
    assert_team_marked_row(&harness).await;
    assert_team_clean_and_string_rows(&harness).await;
    assert_vantage_row(&harness).await;
    assert_environment_and_cloudzero_rows(&harness).await;
    assert_sso_and_token_rows(&harness).await;
}

fn assert_report_counts(report: &MigrationReport, rotation: &FakeRotation) {
    let model = location_of(report, "model_table");
    assert_eq!((model.scanned, model.legacy, model.migrated), (1, 1, 0));
    let team = location_of(report, "team.callback_vars");
    assert_eq!((team.scanned, team.migrated, team.already_v2), (2, 1, 1));
    let vantage = location_of(report, "vantage_settings");
    assert_eq!(
        (vantage.scanned, vantage.migrated, vantage.already_v2),
        (2, 1, 1)
    );
    assert_eq!(location_of(report, "cloudzero_settings").migrated, 1);
    let sso = location_of(report, "sso_config");
    assert_eq!((sso.migrated, sso.already_v2, sso.plaintext), (1, 1, 1));
    assert_eq!(
        rotation.calls(),
        [RotationCall {
            key: SALT_KEY.to_owned(),
            actor: Some("admin".to_owned()),
        }]
    );
}

async fn assert_team_marked_row(harness: &Harness) {
    assert_eq!(
        harness
            .json_kind("LiteLLM_TeamTable", "metadata", "team_id", "team-marked")
            .await,
        "string"
    );
    let team_metadata = harness
        .json_content("LiteLLM_TeamTable", "metadata", "team_id", "team-marked")
        .await;
    let sealed = callback_text(&team_metadata, "api_key");
    let inner = sealed
        .strip_prefix("litellm_enc::")
        .expect("the migrated value keeps the marker");
    assert!(inner.starts_with("v2:gcm:"));
    assert_eq!(open(SALT_KEY, inner).as_deref(), Some("sk-team"));
    assert_ne!(
        harness
            .updated_at("LiteLLM_TeamTable", "team_id", "team-marked")
            .await,
        SEED_TIME
    );
}

async fn assert_team_clean_and_string_rows(harness: &Harness) {
    assert_eq!(
        harness
            .json_kind("LiteLLM_TeamTable", "metadata", "team_id", "team-clean")
            .await,
        "object"
    );
    assert_eq!(
        harness
            .updated_at("LiteLLM_TeamTable", "team_id", "team-clean")
            .await,
        SEED_TIME
    );
    assert_eq!(
        harness
            .json_kind("LiteLLM_TeamTable", "metadata", "team_id", "team-string")
            .await,
        "string"
    );
}

async fn assert_vantage_row(harness: &Harness) {
    assert_eq!(
        harness
            .json_kind(
                "LiteLLM_Config",
                "param_value",
                "param_name",
                "vantage_settings"
            )
            .await,
        "string"
    );
    let vantage_value = harness
        .json_content(
            "LiteLLM_Config",
            "param_value",
            "param_name",
            "vantage_settings",
        )
        .await;
    let api_key = field_text(&vantage_value, "api_key").expect("api_key stays text");
    assert!(api_key.starts_with("v2:gcm:"));
    assert_eq!(open(SALT_KEY, &api_key).as_deref(), Some("sk-vantage"));
    assert_eq!(
        field_text(&vantage_value, "integration_token").as_deref(),
        Some(v2(SALT_KEY, "token-vantage").as_str())
    );
}

async fn assert_environment_and_cloudzero_rows(harness: &Harness) {
    assert_eq!(
        harness
            .json_kind(
                "LiteLLM_Config",
                "param_value",
                "param_name",
                "environment_variables"
            )
            .await,
        "object"
    );
    let cloudzero = harness
        .json_content(
            "LiteLLM_Config",
            "param_value",
            "param_name",
            "cloudzero_settings",
        )
        .await;
    let cloudzero_key = field_text(&cloudzero, "api_key").expect("api_key stays text");
    assert!(cloudzero_key.starts_with("v2:gcm:"));
    assert_eq!(
        open(SALT_KEY, &cloudzero_key).as_deref(),
        Some("sk-cloudzero")
    );
}

async fn assert_sso_and_token_rows(harness: &Harness) {
    assert_eq!(
        harness
            .json_kind("LiteLLM_SSOConfig", "sso_settings", "id", "sso_config")
            .await,
        "string"
    );
    let sso_value = harness
        .json_content("LiteLLM_SSOConfig", "sso_settings", "id", "sso_config")
        .await;
    let client_secret = field_text(&sso_value, "client_secret").expect("client_secret stays text");
    assert!(client_secret.starts_with("v2:gcm:"));
    assert_eq!(
        open(SALT_KEY, &client_secret).as_deref(),
        Some("sso-secret")
    );
    assert_eq!(
        field_text(&sso_value, "client_id").as_deref(),
        Some("plain-client")
    );
    assert_ne!(
        harness
            .updated_at("LiteLLM_SSOConfig", "id", "sso_config")
            .await,
        SEED_TIME
    );
    assert_eq!(
        harness
            .updated_at("LiteLLM_VerificationToken", "token", "token-clean")
            .await,
        SEED_TIME
    );
}

#[tokio::test]
async fn partial_failure_keeps_the_earlier_rows_written() {
    let harness = Harness::start(vec![SCHEMA.to_owned(), FAIL_CLOUDZERO_WRITE.to_owned()]).await;
    seed(&harness).await;
    let mut rotation = FakeRotation::default();
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let error = real_run(&harness, &mut rotation, &cipher, None)
        .await
        .expect_err("the failed cloudzero write stops the migration");
    assert!(matches!(
        error,
        MigrationError::Store(MigrationStoreError::Database(_))
    ));
    assert_earlier_rows_written(&harness).await;
    assert_cloudzero_and_sso_kept(&harness).await;
}

async fn assert_earlier_rows_written(harness: &Harness) {
    assert_eq!(
        harness
            .json_kind("LiteLLM_TeamTable", "metadata", "team_id", "team-marked")
            .await,
        "string"
    );
    assert_eq!(
        harness
            .json_kind(
                "LiteLLM_Config",
                "param_value",
                "param_name",
                "vantage_settings"
            )
            .await,
        "string"
    );
}

async fn assert_cloudzero_and_sso_kept(harness: &Harness) {
    assert_eq!(
        harness
            .json_kind(
                "LiteLLM_Config",
                "param_value",
                "param_name",
                "cloudzero_settings"
            )
            .await,
        "string"
    );
    let cloudzero = harness
        .json_content(
            "LiteLLM_Config",
            "param_value",
            "param_name",
            "cloudzero_settings",
        )
        .await;
    let cloudzero_key = field_text(&cloudzero, "api_key").expect("api_key stays text");
    assert!(!cloudzero_key.starts_with("v2:gcm:"));
    assert_eq!(
        open(SALT_KEY, &cloudzero_key).as_deref(),
        Some("sk-cloudzero")
    );
    assert_eq!(
        harness
            .json_kind("LiteLLM_SSOConfig", "sso_settings", "id", "sso_config")
            .await,
        "object"
    );
}

#[tokio::test]
async fn repeat_migration_skips_migrated_rows_and_writes_nothing() {
    let harness = Harness::start(vec![SCHEMA.to_owned()]).await;
    seed(&harness).await;
    let cipher = cipher(Algorithm::Aes256Gcm, Some(SALT_KEY));
    let mut first = FakeRotation::default();
    real_run(&harness, &mut first, &cipher, None)
        .await
        .expect("the first run migrates the walker locations");
    let after_first = harness.snapshot().await;
    let mut second = FakeRotation::default();
    let report = real_run(&harness, &mut second, &cipher, None)
        .await
        .expect("the second run only reports");
    assert_eq!(harness.snapshot().await, after_first);
    assert_second_run_counts(&report);
}

fn assert_second_run_counts(report: &MigrationReport) {
    let team = location_of(report, "team.callback_vars");
    assert_eq!((team.scanned, team.migrated, team.already_v2), (1, 0, 1));
    let vantage = location_of(report, "vantage_settings");
    assert_eq!(
        (vantage.scanned, vantage.migrated, vantage.already_v2),
        (2, 0, 2)
    );
    let cloudzero = location_of(report, "cloudzero_settings");
    assert_eq!(
        (cloudzero.scanned, cloudzero.migrated, cloudzero.already_v2),
        (1, 0, 1)
    );
    let sso = location_of(report, "sso_config");
    assert_eq!((sso.scanned, sso.already_v2, sso.plaintext), (2, 1, 1));
    let model = location_of(report, "model_table");
    assert_eq!((model.scanned, model.legacy, model.migrated), (1, 1, 0));
}

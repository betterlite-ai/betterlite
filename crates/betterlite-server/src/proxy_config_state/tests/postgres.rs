use std::collections::{BTreeMap, BTreeSet};
use std::future::ready;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::settings::{JsonValue, Section};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_db::table_ops::{ConfigParamInvalidator, InvalidateFuture};
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredValue,
};
use clap::Parser as _;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::sleep;
use tokio_postgres::NoTls;

use super::support::{self, json};
use crate::proxy_config_state::database::postgres::{
    CONFIG_SECTION_LOCK_SQL, READ_CONFIG_SQL, UPSERT_CONFIG_SQL, decode_section_values,
    encode_config_value,
};
use crate::proxy_config_state::database::{
    ConfigStateDatabase as _, DatabaseConfigState, SectionWrite,
};

const CONFIG_SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_Config" (
    "param_name" TEXT NOT NULL,
    "param_value" JSONB,
    CONSTRAINT "LiteLLM_Config_pkey" PRIMARY KEY ("param_name")
);
"#;

const ADVISORY_WAITERS_SQL: &str = "SELECT count(*) FROM pg_locks \
     WHERE locktype = 'advisory' AND NOT granted \
     AND database = (SELECT oid FROM pg_database WHERE datname = current_database())";

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

// The config row stores one JSON text inside another, so a seeded and a written row read back through the decoder
struct Database {
    fixture: PostgresFixture,
    pool: Pool,
    cipher: DbCipher,
}

impl Database {
    async fn start() -> Self {
        let configured = Environment::parse_from(["proxy-config-state-tests"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            configured.as_ref(),
            vec![CONFIG_SCHEMA.to_owned()],
        )
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
            .max_size(4)
            .build()
            .expect("the fixture pool builds");
        Self {
            fixture,
            pool,
            cipher: cipher(),
        }
    }

    async fn seed(&self, section: Section, values: &BTreeMap<String, JsonValue>) {
        self.seed_named(section.name(), values).await;
    }

    async fn seed_named(&self, name: &str, values: &BTreeMap<String, JsonValue>) {
        let object = JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
        let encoded = encode_config_value(&object).expect("the seed row encodes");
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(UPSERT_CONFIG_SQL, &[&name, &encoded])
            .await
            .expect("the seed row commits");
    }

    async fn stored(&self, section: Section) -> Option<BTreeMap<String, JsonValue>> {
        self.stored_named(section.name()).await
    }

    async fn stored_named(&self, name: &str) -> Option<BTreeMap<String, JsonValue>> {
        let client = self.pool.get().await.expect("a fixture connection");
        let row = client
            .query_opt(READ_CONFIG_SQL, &[&name])
            .await
            .expect("the stored row reads");
        row.map(|stored_row| {
            let text: Option<String> = stored_row.get(0);
            decode_section_values(Section::GeneralSettings, &text.expect("the row holds text"))
                .expect("the stored row decodes")
        })
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

#[derive(Default)]
struct RecordingInvalidator {
    names: Mutex<Vec<String>>,
}

impl RecordingInvalidator {
    fn names(&self) -> Vec<String> {
        self.names.lock().expect("invalidations").clone()
    }
}

impl ConfigParamInvalidator for RecordingInvalidator {
    fn invalidate_param<'call>(&'call self, param_name: &'call str) -> InvalidateFuture<'call> {
        self.names
            .lock()
            .expect("invalidations")
            .push(param_name.to_owned());
        Box::pin(ready(()))
    }
}

fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("sk-config-test".to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn decrypt_policy() -> DecryptPolicy {
    DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Original,
    }
}

fn decrypted_plaintext(database: &Database, sealed: &BTreeMap<String, JsonValue>) -> String {
    let Some(JsonValue::String(text)) = sealed.get("OPENAI_API_KEY") else {
        panic!("the sealed key stays a string");
    };
    let opened = database
        .cipher
        .decrypt_value(
            StoredValue::<()>::Text(SecretString::from(text.clone())),
            "OPENAI_API_KEY",
            decrypt_policy(),
        )
        .expect("the sealed value decrypts");
    let StoredValue::Text(plaintext) = opened else {
        panic!("the sealed value stays text");
    };
    plaintext.expose_secret().to_owned()
}

async fn wait_for_an_advisory_waiter(client: &Client) {
    for _poll in 0_u32..400 {
        let waiting: i64 = client
            .query_one(ADVISORY_WAITERS_SQL, &[])
            .await
            .expect("the lock waiter reads")
            .get(0);
        if waiting > 0 {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("the config save never waited on the section lock");
}

#[tokio::test]
async fn database_seal_environment_encrypts_every_value() {
    let database = Database::start().await;
    let store = PostgresStore {
        pool: Some(database.pool.clone()),
    };
    let invalidator = RecordingInvalidator::default();
    let state = DatabaseConfigState {
        store: &store,
        cipher: &database.cipher,
        invalidator: &invalidator,
    };
    let values = support::values(&[
        ("OPENAI_API_KEY", json!("sk-plain")),
        ("OTHER_KEY", json!("other-plain")),
    ]);
    let sealed = state.seal_environment(&values).expect("the values seal");
    assert_eq!(
        sealed.keys().collect::<Vec<&String>>(),
        values.keys().collect::<Vec<&String>>()
    );
    assert_ne!(
        sealed.get("OPENAI_API_KEY"),
        values.get("OPENAI_API_KEY"),
        "OPENAI_API_KEY stays plaintext"
    );
    assert_ne!(
        sealed.get("OTHER_KEY"),
        values.get("OTHER_KEY"),
        "OTHER_KEY stays plaintext"
    );
    assert_eq!(decrypted_plaintext(&database, &sealed), "sk-plain");
    database.close().await;
}

#[tokio::test]
async fn database_merge_section_reads_and_writes_the_stored_row() {
    let database = Database::start().await;
    database
        .seed(
            Section::GeneralSettings,
            &support::values(&[("kept", json!(1))]),
        )
        .await;
    let store = PostgresStore {
        pool: Some(database.pool.clone()),
    };
    let invalidator = RecordingInvalidator::default();
    let state = DatabaseConfigState {
        store: &store,
        cipher: &database.cipher,
        invalidator: &invalidator,
    };
    let write = SectionWrite {
        section: Section::GeneralSettings,
        changed: support::values(&[("added", json!(2))]),
        removed: BTreeSet::new(),
    };
    let written = state
        .merge_section(&write)
        .await
        .expect("the section merges");
    let merged = support::values(&[("added", json!(2)), ("kept", json!(1))]);
    assert_eq!(written, Some(merged.clone()));
    assert_eq!(
        database.stored(Section::GeneralSettings).await,
        Some(merged)
    );
    assert_eq!(invalidator.names(), vec!["general_settings"]);
    database.close().await;
}

#[tokio::test]
async fn database_merge_section_leaves_a_matching_row_and_the_cache_alone() {
    let database = Database::start().await;
    database
        .seed(
            Section::GeneralSettings,
            &support::values(&[("kept", json!(1))]),
        )
        .await;
    let store = PostgresStore {
        pool: Some(database.pool.clone()),
    };
    let invalidator = RecordingInvalidator::default();
    let state = DatabaseConfigState {
        store: &store,
        cipher: &database.cipher,
        invalidator: &invalidator,
    };
    let write = SectionWrite {
        section: Section::GeneralSettings,
        changed: support::values(&[("kept", json!(1))]),
        removed: BTreeSet::new(),
    };
    let written = state
        .merge_section(&write)
        .await
        .expect("the section merges");
    assert_eq!(written, None);
    assert!(invalidator.names().is_empty());
    assert_eq!(
        database.stored(Section::GeneralSettings).await,
        Some(support::values(&[("kept", json!(1))]))
    );
    database.close().await;
}

#[tokio::test]
async fn database_merge_environment_variables_keeps_unnamed_keys() {
    let database = Database::start().await;
    database
        .seed(
            Section::EnvironmentVariables,
            &support::values(&[("keep", json!("old")), ("drop", json!("gone"))]),
        )
        .await;
    let store = PostgresStore {
        pool: Some(database.pool.clone()),
    };
    let invalidator = RecordingInvalidator::default();
    let state = DatabaseConfigState {
        store: &store,
        cipher: &database.cipher,
        invalidator: &invalidator,
    };
    let sealed = support::values(&[("keep", json!("new")), ("add", json!("sealed:y"))]);
    let deleted = BTreeSet::from(["drop".to_owned()]);
    state
        .merge_environment_variables(&sealed, &deleted)
        .await
        .expect("the environment row merges");
    assert_eq!(
        database.stored(Section::EnvironmentVariables).await,
        Some(support::values(&[
            ("add", json!("sealed:y")),
            ("keep", json!("new")),
        ]))
    );
    assert_eq!(invalidator.names(), vec!["environment_variables"]);
    database.close().await;
}

#[tokio::test]
async fn database_insert_config_upserts_each_row_and_invalidates_its_name() {
    let database = Database::start().await;
    let store = PostgresStore {
        pool: Some(database.pool.clone()),
    };
    let invalidator = RecordingInvalidator::default();
    let state = DatabaseConfigState {
        store: &store,
        cipher: &database.cipher,
        invalidator: &invalidator,
    };
    let rows = vec![("custom_row".to_owned(), json!({"x": 1}))];
    state.insert_config(&rows).await.expect("the rows insert");
    assert_eq!(
        database.stored_named("custom_row").await,
        Some(support::values(&[("x", json!(1))]))
    );
    assert_eq!(invalidator.names(), vec!["custom_row"]);
    database.close().await;
}

#[tokio::test]
async fn database_merge_section_waits_for_the_section_lock() {
    let database = Database::start().await;
    let mut holder = database.pool.get().await.expect("a holder connection");
    let held = holder.transaction().await.expect("the holder transaction");
    held.execute(CONFIG_SECTION_LOCK_SQL, &[&Section::GeneralSettings.name()])
        .await
        .expect("the holder takes the section lock");
    let prober = database.pool.get().await.expect("a prober connection");
    let store = PostgresStore {
        pool: Some(database.pool.clone()),
    };
    let invalidator = RecordingInvalidator::default();
    let state = DatabaseConfigState {
        store: &store,
        cipher: &database.cipher,
        invalidator: &invalidator,
    };
    let write = SectionWrite {
        section: Section::GeneralSettings,
        changed: support::values(&[("waited", json!(true))]),
        removed: BTreeSet::new(),
    };
    let release = async {
        wait_for_an_advisory_waiter(&prober).await;
        held.commit().await.expect("the holder commits");
    };
    let (written, ()) = tokio::join!(state.merge_section(&write), release);
    assert_eq!(
        written.expect("the section merges"),
        Some(support::values(&[("waited", json!(true))]))
    );
    assert_eq!(
        database.stored(Section::GeneralSettings).await,
        Some(support::values(&[("waited", json!(true))]))
    );
    database.close().await;
}

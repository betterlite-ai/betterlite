#![cfg(test)]

use std::future::ready;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::settings::{JsonValue, Section};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_db::table_ops::{ConfigParamInvalidator, InvalidateFuture};
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::super::SettingsMap;
use super::super::store::{ConfigUpdateStore as _, PostgresConfigUpdateStore};
use super::support::{parsed, settings};

const CONFIG_SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_Config" (
    "param_name" TEXT NOT NULL,
    "param_value" JSONB,
    CONSTRAINT "LiteLLM_Config_pkey" PRIMARY KEY ("param_name")
);
"#;

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
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
            signing_key: Some(SecretString::from("sk-config-update-test".to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

async fn fixture_pool() -> (PostgresFixture, Pool) {
    let configured = Environment::parse_from(["config-update-tests"])
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
        .max_size(2)
        .build()
        .expect("the fixture pool builds");
    (fixture, pool)
}

#[tokio::test]
async fn the_postgres_store_reads_writes_and_seals_sections() {
    let (fixture, pool) = fixture_pool().await;
    let invalidator = Arc::new(RecordingInvalidator::default());
    let invalidator_arc = Arc::clone(&invalidator);
    let invalidator_handle: Arc<dyn ConfigParamInvalidator> = invalidator_arc;
    let store = PostgresConfigUpdateStore {
        store: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
        cipher: Arc::new(cipher()),
        invalidator: invalidator_handle,
    };
    let sent = settings(&[("OPENAI_API_KEY", JsonValue::String("sk-plain".to_owned()))]);
    let sealed = store.seal_environment(&sent).expect("the values seal");
    assert_eq!(
        sealed.keys().collect::<Vec<&String>>(),
        sent.keys().collect::<Vec<&String>>(),
        "the sealed map keeps every sent key"
    );
    assert_ne!(
        sealed.get("OPENAI_API_KEY"),
        Some(&JsonValue::String("sk-plain".to_owned())),
        "the sealed value stays plaintext"
    );
    assert_eq!(
        store
            .read_section(Section::GeneralSettings)
            .await
            .expect("the missing section reads"),
        SettingsMap::new()
    );
    let values = settings(&[("added", parsed("2"))]);
    store
        .upsert_section(Section::GeneralSettings, &values)
        .await
        .expect("the section upserts");
    assert_eq!(invalidator.names(), vec!["general_settings"]);
    assert_eq!(
        store
            .read_section(Section::GeneralSettings)
            .await
            .expect("the stored section reads"),
        values
    );
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}

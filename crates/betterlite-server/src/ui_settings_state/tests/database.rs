use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Config, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::support::stored;
use crate::ui_settings_state::repository::{DatabaseUiSettingsRepository, UiSettingsRepository};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_UISettings" (
    id TEXT PRIMARY KEY,
    ui_settings JSONB,
    updated_at TIMESTAMPTZ DEFAULT now()
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

#[tokio::test]
async fn database_repository_reads_the_ui_settings_row() {
    let url = Environment::parse_from(["ui-settings-state-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let repository = DatabaseUiSettingsRepository {
        store: &store,
        availability: &availability,
    };
    assert_eq!(repository.stored_ui_settings().await.unwrap(), None);
    let client = pool.get().await.unwrap();
    client
        .execute(
            r#"INSERT INTO "LiteLLM_UISettings" (id, ui_settings) VALUES ($1, $2::jsonb)"#,
            &[&"user_banner", &r#"{"enable_chat_ui": false}"#],
        )
        .await
        .unwrap();
    assert_eq!(repository.stored_ui_settings().await.unwrap(), None);
    client
        .execute(
            r#"INSERT INTO "LiteLLM_UISettings" (id, ui_settings) VALUES ($1, $2::jsonb)"#,
            &[&"ui_settings", &r#"{"enable_chat_ui": true}"#],
        )
        .await
        .unwrap();
    assert_eq!(
        repository.stored_ui_settings().await.unwrap(),
        Some(stored(r#"{"enable_chat_ui": true}"#))
    );
    pool.close();
    fixture.close_async().await.unwrap();
}

use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::support::settings;
use crate::ui_settings_state::decode::decode_stored_ui_settings;
use crate::ui_settings_state::repository::{DatabaseUiSettingsRepository, UiSettingsRepository};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_UISettings" (
    id TEXT PRIMARY KEY,
    ui_settings JSONB,
    updated_at TIMESTAMPTZ DEFAULT now()
);
"#;

const INSERT_ROWS: &str =
    r#"INSERT INTO "LiteLLM_UISettings" (id, ui_settings) VALUES ($1, $2::jsonb), ($3, $4::jsonb)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

#[tokio::test]
async fn database_repository_reads_the_ui_settings_row() {
    let environment = Environment::parse_from(["ui-settings-state-tests"]);
    let url = environment.url.map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
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
            INSERT_ROWS,
            &[
                &"user_banner",
                &"{}",
                &"ui_settings",
                &r#"{"enable_chat_ui": true}"#,
            ],
        )
        .await
        .unwrap();
    drop(client);
    let found = repository.stored_ui_settings().await.unwrap();
    let text = found.expect("the row exists");
    assert_eq!(
        decode_stored_ui_settings(text.as_bytes()).unwrap(),
        settings(r#"{"enable_chat_ui": true}"#)
    );
    fixture.close_async().await.unwrap();
}

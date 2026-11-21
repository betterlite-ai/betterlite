use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::sql::BASELINE;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::model_effective_fields::error::ModelFieldError;
use crate::model_effective_fields::stored::get_db_model;

use super::support::{cipher, object, sealed};

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const INSERT: &str = r#"INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, model_info, created_by, updated_by) VALUES ($1, $2, ($3::text)::jsonb, ($4::text)::jsonb, 'admin', 'admin')"#;

fn repository<'db>(
    store: &'db PostgresStore,
    availability: &'db Availability,
) -> ModelRepository<'db> {
    let client = PrismaClient {
        writer: store,
        reader: store,
        availability,
    };
    ModelRepository::new(
        Some(client),
        redis_config_change_publisher::<TableError>(None),
        None,
        cipher(),
    )
}

#[tokio::test]
async fn get_db_model_mirrors_custom_prices_onto_model_info() {
    let url = Environment::parse_from(["model-effective-fields-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![BASELINE.to_owned()])
            .resolve()
            .await
            .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    let model = sealed("gpt-4o");
    let unpriced =
        format!(r#"{{"model": "{model}", "input_cost_per_token": 0.5, "tiered_pricing": null}}"#);
    client
        .execute(INSERT, &[&"a", &"alpha", &unpriced, &None::<&str>])
        .await
        .unwrap();
    let repriced = r#"{"model": "gpt-4o", "output_cost_per_token": 0.2}"#;
    let info = r#"{"id": "b", "output_cost_per_token": 0.1, "member_auto_router": true}"#;
    client
        .execute(INSERT, &[&"b", &"beta", &repriced, &Some(info)])
        .await
        .unwrap();
    drop(client);
    let store = PostgresStore { pool: Some(pool) };
    let availability = Availability::default();
    let models = repository(&store, &availability);

    let alpha = get_db_model("a", &models).await.unwrap().unwrap();
    assert_eq!(alpha.model_name, "alpha");
    assert_eq!(alpha.litellm_params, object(&unpriced));
    assert_eq!(alpha.model_info, object(r#"{"input_cost_per_token": 0.5}"#));
    assert!(!alpha.blocked);
    let beta = get_db_model("b", &models).await.unwrap().unwrap();
    assert_eq!(
        beta.model_info,
        object(r#"{"id": "b", "output_cost_per_token": 0.2, "member_auto_router": true}"#)
    );
    assert_eq!(get_db_model("missing", &models).await.unwrap(), None);
}

#[tokio::test]
async fn get_db_model_without_a_database_fails() {
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let models = repository(&store, &availability);
    let error = get_db_model("a", &models).await.unwrap_err();
    assert!(
        matches!(
            error,
            ModelFieldError::Database(_) | ModelFieldError::Table(_)
        ),
        "{error:?}"
    );
    assert_eq!(error.status(), 500);
}

use std::sync::Arc;

use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::TableActions as _;

use crate::auxiliary_repository::repository::TableError;
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::DbError;
use crate::resource_repository::budget::BudgetRepository;
use crate::resource_repository::config::{
    ConfigDecodeError, ConfigError, ConfigRepository, MAX_CONFIG_VALUE_BYTES,
};
use crate::resource_repository::credential::{
    CredentialError, CredentialsRepository, credential_from_row,
};
use crate::resource_repository::model::{ModelError, ModelRepository};
use crate::resource_rows::ResourceRowsError;

use super::support::{
    Case, Journal, NO_DB, batch, captured_logs, cipher, dead_redis, failing_publisher, field,
    insert_config, insert_credential, insert_model, observing_publisher, recording_publisher,
    value, write, written,
};

pub(crate) async fn missing_client_before_query(_case: &Case) {
    let journal = Journal::default();
    let model = ModelRepository::new(None, recording_publisher(&journal), None, cipher());
    let error = model.find_all_except("model").await.unwrap_err();
    assert_eq!(error.to_string(), NO_DB);
    assert!(matches!(
        error,
        ModelError::Database(DbError::NoDbConnected)
    ));
    assert_eq!(model.prisma_client().err().unwrap().to_string(), NO_DB);
    let config = ConfigRepository::new(None, false);
    assert_eq!(
        config.get_param("param").await.unwrap_err().to_string(),
        NO_DB
    );
    assert_eq!(config.prisma_client().err().unwrap().to_string(), NO_DB);
    let credentials = CredentialsRepository::new(None, recording_publisher(&journal));
    assert_eq!(credentials.find_all().await.unwrap_err().to_string(), NO_DB);
    assert_eq!(
        credentials.prisma_client().err().unwrap().to_string(),
        NO_DB
    );
    let budget = BudgetRepository::new(None);
    assert_eq!(
        budget.find_by_id("budget").await.unwrap_err().to_string(),
        NO_DB
    );
    assert!(journal.entries().is_empty());
}

pub(crate) async fn invalid_model_json(case: &Case) {
    insert_model(case, "invalid-model", "invalid", "\"not-an-object\"", None).await;
    let repository = ModelRepository::new(
        Some(case.client()),
        redis_config_change_publisher(None),
        None,
        cipher(),
    );
    let error = repository.find_all_except("absent").await.unwrap_err();
    assert!(matches!(error, ModelError::Row(ResourceRowsError::Json(_))));
    batch(
        &case.writer,
        "DELETE FROM \"LiteLLM_ProxyModelTable\" WHERE model_id = 'invalid-model'",
    )
    .await;
}

pub(crate) async fn config_bare_string_read_failure(case: &Case) {
    insert_config(&case.writer, "config-bare-failure", "abc").await;
    let repository = ConfigRepository::new(Some(case.client()), false);
    let error = repository
        .get_param("config-bare-failure")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Json(ConfigDecodeError::Json(_))
    ));
    let deep = format!("{}{}", "[".repeat(65), "]".repeat(65));
    insert_config(&case.writer, "config-deep-failure", &deep).await;
    let error = repository
        .get_param("config-deep-failure")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Json(ConfigDecodeError::TooDeep)
    ));
    let long = "a".repeat(MAX_CONFIG_VALUE_BYTES + 1);
    insert_config(&case.writer, "config-long-failure", &long).await;
    let error = repository
        .get_param("config-long-failure")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Json(ConfigDecodeError::TooLarge { len }) if len == MAX_CONFIG_VALUE_BYTES + 1
    ));
}

pub(crate) async fn config_upsert_failure(case: &Case) {
    batch(&case.writer, "DROP TABLE \"LiteLLM_Config\"").await;
    let repository = ConfigRepository::new(Some(case.client()), false);
    let error = repository
        .set_param("config-dropped", JsonData::Null)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Table(TableError::Database(DbError::SqlState { .. }))
    ));
}

pub(crate) async fn credential_missing_name(_case: &Case) {
    let missing = credential_from_row(value(r#"{"credential_id":"shape-only"}"#)).unwrap_err();
    assert!(matches!(missing, CredentialError::MissingName));
    assert_eq!(
        missing.to_string(),
        "stored credential row has no credential_name"
    );
    let not_object = credential_from_row(JsonData::Null).unwrap_err();
    assert!(matches!(
        not_object,
        CredentialError::Row(ResourceRowsError::NotAnObject)
    ));
}

pub(crate) async fn model_raw_write_before_publication(case: &Case) {
    let journal = Journal::default();
    let sql = "SELECT 1 FROM \"LiteLLM_ProxyModelTable\" WHERE model_name = 'observed-model'";
    let publish = observing_publisher(&journal, &case.writer, sql.to_owned());
    let repository = ModelRepository::new(Some(case.client()), publish, None, cipher());
    let created = repository
        .table()
        .unwrap()
        .create(write(&[
            ("model_id", value("\"observed-model-id\"")),
            ("model_name", value("\"observed-model\"")),
            ("litellm_params", value(r#"{"api_key":"k"}"#)),
            ("created_by", value("\"seed\"")),
            ("updated_by", value("\"seed\"")),
        ]))
        .await
        .unwrap();
    assert_eq!(field(&created, "model_name"), &value("\"observed-model\""));
    assert_eq!(
        journal.entries(),
        vec!["publish:litellm_proxymodeltable:visible=true".to_owned()]
    );
}

pub(crate) async fn credential_write_before_publication(case: &Case) {
    let journal = Journal::default();
    let sql = "SELECT 1 FROM \"LiteLLM_CredentialsTable\" WHERE credential_name = 'cred-observed'";
    let publish = observing_publisher(&journal, &case.writer, sql.to_owned());
    let repository = CredentialsRepository::new(Some(case.client()), publish);
    let created = repository
        .create(written(
            r#"{"credential_name":"cred-observed","credential_values":{"api_key":"k"},"created_by":"seed","updated_by":"seed"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(
        field(&created, "credential_name"),
        &value("\"cred-observed\"")
    );
    assert_eq!(
        journal.entries(),
        vec!["publish:litellm_credentialstable:visible=true".to_owned()]
    );
}

pub(crate) async fn credential_redis_absent_noop(case: &Case) {
    let repository =
        CredentialsRepository::new(Some(case.client()), redis_config_change_publisher(None));
    let created = repository
        .create(written(
            r#"{"credential_name":"cred-no-redis","credential_values":{"api_key":"k"},"created_by":"seed","updated_by":"seed"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(
        field(&created, "credential_name"),
        &value("\"cred-no-redis\"")
    );
    assert!(
        repository
            .find_by_name("cred-no-redis")
            .await
            .unwrap()
            .is_some()
    );
}

pub(crate) async fn credential_redis_failure_warns_but_succeeds(case: &Case) {
    let (logs, _guard) = captured_logs();
    let cache = Arc::new(dead_redis().await);
    let publish = redis_config_change_publisher(Some(cache));
    let repository = CredentialsRepository::new(Some(case.client()), publish);
    let created = repository
        .create(written(
            r#"{"credential_name":"cred-dead-redis","credential_values":{"api_key":"k"},"created_by":"seed","updated_by":"seed"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(
        field(&created, "credential_name"),
        &value("\"cred-dead-redis\"")
    );
    assert!(
        logs.text()
            .contains("config sync publish for litellm_credentialstable failed: ")
    );
    assert!(
        repository
            .find_by_name("cred-dead-redis")
            .await
            .unwrap()
            .is_some()
    );
}

pub(crate) async fn injected_publisher_error_after_commit(case: &Case) {
    let repository =
        CredentialsRepository::new(Some(case.client()), failing_publisher("publish boom"));
    let error = repository
        .create(written(
            r#"{"credential_name":"cred-injected","credential_values":{"api_key":"k"},"created_by":"seed","updated_by":"seed"}"#,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CredentialError::Table(TableError::Publish { ref message }) if message == "publish boom"
    ));
    assert!(
        repository
            .find_by_name("cred-injected")
            .await
            .unwrap()
            .is_some()
    );
}

pub(crate) async fn read_has_no_publication(case: &Case) {
    insert_credential(
        case,
        "cred-read-id",
        "cred-read",
        r#"{"api_key":"k"}"#,
        None,
    )
    .await;
    insert_model(case, "read-model", "read", r#"{"api_key":"k"}"#, None).await;
    let journal = Journal::default();
    let credentials =
        CredentialsRepository::new(Some(case.client()), recording_publisher(&journal));
    credentials.find_all().await.unwrap();
    credentials
        .find_by_name("cred-read")
        .await
        .unwrap()
        .unwrap();
    let model = ModelRepository::new(
        Some(case.client()),
        recording_publisher(&journal),
        None,
        cipher(),
    );
    model.find_all_except("absent").await.unwrap();
    assert!(journal.entries().is_empty());
}

#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{GcsConfigError, logging_config};
use crate::gcs_client::gcs_storage_auth::GcsBucketBase;
use crate::gcs_client::gcs_storage_auth::error::GcsClientError;
use crate::gcs_client::gcs_storage_auth::tests::support::{FakeAuth, Harness, RecordingClient};
use crate::types::gcs_storage_auth::type_definitions::{GcsLoggingConfig, GcsLoggingKwargs};

fn dynamic(pairs: &[(&str, Json)]) -> Json {
    let mut fields = IndexMap::new();
    for (name, value) in pairs {
        fields.insert((*name).to_owned(), value.clone());
    }
    Json::Object(fields)
}

fn kwargs(params: Option<Json>) -> GcsLoggingKwargs {
    GcsLoggingKwargs {
        dynamic_params: params,
    }
}

#[tokio::test]
async fn the_dynamic_bucket_name_wins_over_the_instance_bucket() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let base = harness.build(None);
    let config = logging_config(
        &base,
        &kwargs(Some(dynamic(&[(
            "gcs_bucket_name",
            Json::Text("dyn-bucket".to_owned()),
        )]))),
    )
    .await
    .expect("the dynamic bucket resolves");
    assert_eq!(config.bucket_name, "dyn-bucket");
}

#[tokio::test]
async fn the_instance_bucket_is_used_when_the_dynamic_params_omit_it() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let base = harness.build(None);
    let config = log_config(&base, Some(dynamic(&[]))).await;
    assert_eq!(config.bucket_name, "env-bucket");
}

#[tokio::test]
async fn the_instance_bucket_is_used_when_no_dynamic_params_arrive() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let base = harness.build(None);
    let config = log_config(&base, None).await;
    assert_eq!(config.bucket_name, "env-bucket");
}

#[tokio::test]
async fn a_null_dynamic_bucket_falls_back_to_the_instance_bucket() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let base = harness.build(None);
    let config = log_config(&base, Some(dynamic(&[("gcs_bucket_name", Json::Null)]))).await;
    assert_eq!(config.bucket_name, "env-bucket");
}

#[tokio::test]
async fn the_dynamic_service_account_wins_and_warms_its_own_instance() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let base = harness.build(None);
    let params = dynamic(&[
        ("gcs_bucket_name", Json::Text("dyn-bucket".to_owned())),
        (
            "gcs_path_service_account",
            Json::Text("dyn-account".to_owned()),
        ),
    ]);
    let config = log_config(&base, Some(params)).await;
    assert_eq!(
        config
            .path_service_account
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("dyn-account")
    );
    assert_eq!(
        harness
            .auth
            .last_ensure()
            .credentials
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("dyn-account")
    );
}

#[tokio::test]
async fn the_instance_service_account_is_used_when_the_dynamic_params_omit_it() {
    let harness = Harness::new(&[
        ("GCS_BUCKET_NAME", "env-bucket"),
        ("GCS_PATH_SERVICE_ACCOUNT", "instance-account"),
    ]);
    let base = harness.build(None);
    let config = log_config(&base, Some(dynamic(&[]))).await;
    assert_eq!(
        config
            .path_service_account
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("instance-account")
    );
    assert_eq!(
        harness
            .auth
            .last_ensure()
            .credentials
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("instance-account")
    );
}

#[tokio::test]
async fn a_missing_bucket_name_is_an_error() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let result = logging_config(&base, &GcsLoggingKwargs::default()).await;
    assert!(matches!(result, Err(GcsClientError::MissingBucketName)));
    assert_eq!(harness.auth.ensure_count(), 0);
}

#[tokio::test]
async fn dynamic_params_that_are_not_an_object_are_an_error() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let base = harness.build(None);
    let result = logging_config(&base, &kwargs(Some(Json::Text("params".to_owned())))).await;
    assert!(matches!(
        result,
        Err(GcsClientError::Dynamic(GcsConfigError::Params))
    ));
}

#[tokio::test]
async fn a_dynamic_field_of_the_wrong_type_names_the_field() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let base = harness.build(None);
    let result = logging_config(
        &base,
        &kwargs(Some(dynamic(&[("gcs_bucket_name", Json::Bool(true))]))),
    )
    .await;
    assert!(matches!(
        result,
        Err(GcsClientError::Dynamic(GcsConfigError::Field {
            field: "gcs_bucket_name"
        }))
    ));
}

async fn log_config(
    base: &GcsBucketBase<FakeAuth, RecordingClient>,
    params: Option<Json>,
) -> GcsLoggingConfig<FakeAuth> {
    logging_config(base, &kwargs(params))
        .await
        .expect("the logging configuration resolves")
}

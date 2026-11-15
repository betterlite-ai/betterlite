#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::tests::support::{Answer, dynamic, open};
use super::{ConfigKeyError, config_key, sanitize_config_key};

#[tokio::test]
async fn the_key_uses_the_dynamic_params() {
    let harness = open(&[], Some("instance-bucket"), Answer::ok("{}"));
    let dynamic_params = dynamic(&[
        ("gcs_bucket_name", "dynamic-bucket"),
        ("gcs_path_service_account", "/creds/dynamic.json"),
    ]);
    let key = config_key(&harness.logger, &dynamic_params).expect("the key builds");
    assert_eq!(key, "dynamic-bucket|/creds/dynamic.json");
}

#[tokio::test]
async fn the_key_falls_back_to_the_instance_settings() {
    let harness = open(
        &[
            ("GCS_BUCKET_NAME", "env-bucket"),
            ("GCS_PATH_SERVICE_ACCOUNT", "/creds/instance.json"),
        ],
        None,
        Answer::ok("{}"),
    );
    let key = config_key(&harness.logger, &Json::Null).expect("the key builds");
    assert_eq!(key, "env-bucket|/creds/instance.json");
}

#[tokio::test]
async fn the_key_falls_back_to_the_literal_defaults() {
    let harness = open(&[], None, Answer::ok("{}"));
    let key = config_key(&harness.logger, &Json::Object(IndexMap::new())).expect("the key builds");
    assert_eq!(key, "default|default");
}

#[tokio::test]
async fn a_null_field_falls_back_to_the_instance_settings() {
    let harness = open(&[("GCS_BUCKET_NAME", "env-bucket")], None, Answer::ok("{}"));
    let dynamic_params = Json::Object(IndexMap::from([
        ("gcs_bucket_name".to_owned(), Json::Null),
        ("gcs_path_service_account".to_owned(), Json::Null),
    ]));
    let key = config_key(&harness.logger, &dynamic_params).expect("the key builds");
    assert_eq!(key, "env-bucket|default");
}

#[tokio::test]
async fn a_non_string_field_is_refused() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let dynamic_params = Json::Object(IndexMap::from([(
        "gcs_path_service_account".to_owned(),
        Json::Bool(true),
    )]));
    let error =
        config_key(&harness.logger, &dynamic_params).expect_err("the account must be a string");
    assert!(
        matches!(error, ConfigKeyError::Field { field } if field == "gcs_path_service_account")
    );
}

#[rstest]
#[case::boolean(Json::Bool(true))]
#[case::text(Json::Text("bucket".to_owned()))]
#[case::list(Json::List(Vec::new()))]
#[tokio::test]
async fn a_non_object_dynamic_params_object_is_refused(#[case] dynamic_params: Json) {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let error =
        config_key(&harness.logger, &dynamic_params).expect_err("the params must be an object");
    assert!(matches!(error, ConfigKeyError::Params));
}

#[test]
fn sanitization_hashes_the_key_into_eight_hex_digits() {
    assert_eq!(sanitize_config_key("audit|default"), "config-99a16a92");
    assert_eq!(
        sanitize_config_key("other|/creds/a.json"),
        "config-24b24882"
    );
}

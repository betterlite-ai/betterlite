use std::collections::BTreeMap;

use betterlite_openai::api::file_type_records::{
    FileContentProvider, ProviderUploadPayload, ProviderUploadValue, TwoStepFileUploadConfig,
    UploadData, UploadUrlLocation,
};
use rstest::rstest;

use crate::api::file_contracts::config::{BaseFilesConfig as _, CreateFileBody, FileParams};
use crate::manus_files::create::upload_plan;
use crate::manus_files::error::{
    FILE_DATA_REQUIRED_MESSAGE, MISSING_API_KEY_MESSAGE, ManusFilesError,
};
use crate::manus_files::{
    API_KEY_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE, MANUS_API_KEY_ENV, UPLOAD_URL_KEY,
};

use super::support::{NOW, config, create_file, environment, globals, params, plain_config};

fn planned(
    litellm_params: &str,
    global: Option<&str>,
    environment_pairs: &[(&str, &str)],
) -> TwoStepFileUploadConfig {
    let config = config(globals(None, global), environment(environment_pairs), NOW);
    let file = create_file(Some("doc.pdf"), b"hello");
    upload_plan(&config, Some(&file.file), &params(litellm_params)).expect("the plan builds")
}

#[test]
fn the_plan_is_the_two_step_upload() {
    let built = planned("{}", Some("global-key"), &[]);
    assert_eq!(built.initial_request.method, "POST");
    assert_eq!(built.initial_request.url, "https://api.manus.im/v1/files");
    assert_eq!(
        built.initial_request.headers,
        BTreeMap::from([
            (String::from(API_KEY_HEADER), String::from("global-key")),
            (
                String::from(CONTENT_TYPE_HEADER),
                String::from(JSON_CONTENT_TYPE)
            ),
        ])
    );
    assert_eq!(
        built.initial_request.data,
        UploadData::Provider(ProviderUploadPayload {
            provider: FileContentProvider::Manus,
            value: ProviderUploadValue {
                filename: String::from("doc.pdf"),
            },
        })
    );
    assert_eq!(built.upload_request.method, "PUT");
    assert_eq!(built.upload_request.url, "");
    assert!(built.upload_request.headers.is_empty());
    assert_eq!(
        built.upload_request.data,
        UploadData::Bytes(b"hello".to_vec())
    );
    assert_eq!(built.upload_url_location, UploadUrlLocation::Body);
    assert_eq!(built.upload_url_key, Some(String::from(UPLOAD_URL_KEY)));
}

#[test]
fn the_filename_defaults_to_the_clock() {
    let config = config(globals(None, Some("key")), environment(&[]), 1_700_000_123);
    let file = create_file(None, b"x");
    let built = upload_plan(&config, Some(&file.file), &params("{}")).expect("the plan builds");
    assert!(matches!(
        built.initial_request.data,
        UploadData::Provider(ProviderUploadPayload {
            value: ProviderUploadValue { filename },
            ..
        }) if filename == "file_1700000123"
    ));
}

// docs/compatibility.md: an explicit empty value is used as given, not replaced by the default.
#[test]
fn an_empty_filename_is_used_as_given() {
    let config = config(globals(None, Some("key")), environment(&[]), NOW);
    let file = create_file(Some(""), b"x");
    let built = upload_plan(&config, Some(&file.file), &params("{}")).expect("the plan builds");
    assert!(matches!(
        built.initial_request.data,
        UploadData::Provider(ProviderUploadPayload {
            value: ProviderUploadValue { filename },
            ..
        }) if filename.is_empty()
    ));
}

#[test]
fn a_missing_file_is_refused_before_the_key() {
    let error = upload_plan(&plain_config(NOW), None, &params("{}"))
        .expect_err("without a file there is no plan");
    assert!(matches!(error, ManusFilesError::FileDataRequired));
    assert_eq!(error.to_string(), FILE_DATA_REQUIRED_MESSAGE);
}

#[rstest]
#[case::params_key(
    r#"{"api_key":"params-key"}"#,
    Some("global-key"),
    "env-key",
    "params-key"
)]
#[case::global_key("{}", Some("global-key"), "env-key", "global-key")]
#[case::environment_key("{}", None, "env-key", "env-key")]
#[case::empty_params_key(r#"{"api_key":""}"#, Some("global-key"), "env-key", "global-key")]
fn the_plan_key_follows_the_chain(
    #[case] litellm_params: &str,
    #[case] global: Option<&str>,
    #[case] env_key: &str,
    #[case] expected: &str,
) {
    let built = planned(litellm_params, global, &[(MANUS_API_KEY_ENV, env_key)]);
    assert_eq!(
        built.initial_request.headers.get(API_KEY_HEADER),
        Some(&String::from(expected))
    );
}

#[test]
fn the_plan_without_a_key_names_the_environment_variable() {
    let config = config(globals(None, None), environment(&[]), NOW);
    let file = create_file(Some("doc.pdf"), b"hello");
    let error = upload_plan(&config, Some(&file.file), &params("{}"))
        .expect_err("without a key there is no plan");
    assert!(matches!(error, ManusFilesError::MissingApiKey));
    assert_eq!(error.to_string(), MISSING_API_KEY_MESSAGE);
}

#[test]
fn the_plan_uses_the_litellm_params_api_base() {
    let built = planned(r#"{"api_base":"http://host/v1"}"#, Some("key"), &[]);
    assert_eq!(built.initial_request.url, "http://host/v1/files");
}

#[test]
fn a_null_parameter_counts_as_unset() {
    let built = planned(
        r#"{"api_key":null,"api_base":null}"#,
        Some("global-key"),
        &[],
    );
    assert_eq!(built.initial_request.url, "https://api.manus.im/v1/files");
    assert_eq!(
        built.initial_request.headers.get(API_KEY_HEADER),
        Some(&String::from("global-key"))
    );
}

#[rstest]
#[case::api_key(r#"{"api_key":7}"#, "the api_key parameter must be text")]
#[case::api_base(r#"{"api_base":true}"#, "the api_base parameter must be text")]
fn a_non_text_parameter_is_refused(#[case] litellm_params: &str, #[case] message: &str) {
    let config = config(globals(None, Some("key")), environment(&[]), NOW);
    let file = create_file(Some("doc.pdf"), b"hello");
    let error = upload_plan(&config, Some(&file.file), &params(litellm_params))
        .expect_err("the parameter is not text");
    assert!(matches!(error, ManusFilesError::ParamNotText { .. }));
    assert_eq!(error.to_string(), message);
}

#[test]
fn the_trait_create_request_returns_the_plan() {
    let config = config(globals(None, Some("key")), environment(&[]), NOW);
    let body = config
        .transform_create_file_request(
            "manus-1.6",
            &create_file(Some("a.txt"), b"abc"),
            &FileParams::new(),
            &params("{}"),
        )
        .expect("the plan builds");
    let CreateFileBody::TwoStep(built) = body else {
        panic!("the create body is a two-step plan");
    };
    assert_eq!(
        built.upload_request.data,
        UploadData::Bytes(b"abc".to_vec())
    );
}

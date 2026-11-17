#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Number;

use crate::vertex_file_content::settings::{
    ReadDeployment, ReadEnvironment, ReadSettings, vertex_file_read_settings,
};

fn deployment_of(
    gcs_bucket_name: Option<Json>,
    bucket_name: Option<Json>,
    vertex_credentials: Option<Json>,
) -> ReadDeployment {
    ReadDeployment {
        gcs_bucket_name,
        bucket_name,
        vertex_credentials,
    }
}

fn environment_of(
    batch_bucket_name: Option<&str>,
    bucket_name: Option<&str>,
    path_service_account: Option<&str>,
) -> ReadEnvironment {
    ReadEnvironment {
        batch_bucket_name: batch_bucket_name.map(str::to_owned),
        bucket_name: bucket_name.map(str::to_owned),
        path_service_account: path_service_account.map(SecretString::from),
    }
}

fn text_of(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn bucket(settings: &ReadSettings) -> Option<&str> {
    settings.bucket_name.as_deref()
}

fn service_account(settings: &ReadSettings) -> Option<&str> {
    settings
        .service_account
        .as_ref()
        .map(ExposeSecret::expose_secret)
}

#[test]
fn deployment_bucket() {
    let environment = environment_of(Some("batch-bucket"), Some("env-bucket"), None);
    let both_names = vertex_file_read_settings(
        &deployment_of(Some(text_of("bucket-a")), Some(text_of("bucket-b")), None),
        None,
        &environment,
    );
    assert_eq!(bucket(&both_names), Some("bucket-a"));
    let alias = vertex_file_read_settings(
        &deployment_of(None, Some(text_of("bucket-b")), None),
        None,
        &environment,
    );
    assert_eq!(bucket(&alias), Some("bucket-b"));
    let empty_gcs_name = vertex_file_read_settings(
        &deployment_of(Some(text_of("")), Some(text_of("bucket-b")), None),
        None,
        &environment,
    );
    assert_eq!(bucket(&empty_gcs_name), Some("bucket-b"));
    let empty_alias = vertex_file_read_settings(
        &deployment_of(None, Some(text_of("")), None),
        None,
        &environment,
    );
    assert_eq!(bucket(&empty_alias), Some(""));
    let from_environment =
        vertex_file_read_settings(&deployment_of(None, None, None), None, &environment);
    assert_eq!(bucket(&from_environment), Some("batch-bucket"));
}

#[rstest::rstest]
#[case::truthy_number(Json::Number(Number::from(5_i32)), Some("batch-bucket"))]
#[case::truthy_switch(Json::Bool(true), Some("batch-bucket"))]
#[case::zero_falls_to_the_alias(Json::Number(Number::from(0_i32)), Some("bucket-b"))]
#[case::empty_list_falls_to_the_alias(Json::List(Vec::new()), Some("bucket-b"))]
#[case::empty_object_falls_to_the_alias(Json::Object(IndexMap::new()), Some("bucket-b"))]
fn nonstring_bucket_fallback(#[case] value: Json, #[case] expected: Option<&str>) {
    let environment = environment_of(Some("batch-bucket"), Some("env-bucket"), None);
    let settings = vertex_file_read_settings(
        &deployment_of(Some(value), Some(text_of("bucket-b")), None),
        None,
        &environment,
    );
    assert_eq!(bucket(&settings), expected);
}

#[test]
fn nonstring_bucket_falls_to_the_general_environment() {
    let environment = environment_of(None, Some("env-bucket"), None);
    let value = Json::Number(Number::from(5_i32));
    let settings = vertex_file_read_settings(
        &deployment_of(Some(value), Some(text_of("bucket-b")), None),
        None,
        &environment,
    );
    assert_eq!(bucket(&settings), Some("env-bucket"));
}

#[test]
fn credential_dictionary() {
    let mut fields = IndexMap::new();
    fields.insert(
        "client_email".to_owned(),
        Json::Text("sa@example.iam.gserviceaccount.com".to_owned()),
    );
    fields.insert("private_key".to_owned(), Json::Text("secret".to_owned()));
    let document = Json::Object(fields);
    let argument = Json::Text("argument-sa".to_owned());
    let environment = environment_of(None, None, Some("/env/sa.json"));
    let dictionary = vertex_file_read_settings(
        &deployment_of(None, None, Some(document)),
        Some(&argument),
        &environment,
    );
    assert_eq!(
        service_account(&dictionary),
        Some(r#"{"client_email": "sa@example.iam.gserviceaccount.com", "private_key": "secret"}"#)
    );
    let empty_dictionary = vertex_file_read_settings(
        &deployment_of(None, None, Some(Json::Object(IndexMap::new()))),
        Some(&argument),
        &environment,
    );
    assert_eq!(service_account(&empty_dictionary), Some("argument-sa"));
}

#[test]
fn credential_dictionary_keeps_python_json_text() {
    let mut fields = IndexMap::new();
    fields.insert("flag".to_owned(), Json::Bool(true));
    fields.insert("count".to_owned(), Json::Number(Number::from(2_i32)));
    fields.insert("absent".to_owned(), Json::Null);
    fields.insert("label".to_owned(), Json::Text("a b".to_owned()));
    fields.insert(
        "nested".to_owned(),
        Json::List(vec![Json::Text("a".to_owned()), Json::List(Vec::new())]),
    );
    let settings = vertex_file_read_settings(
        &deployment_of(None, None, Some(Json::Object(fields))),
        None,
        &environment_of(None, None, Some("/env/sa.json")),
    );
    assert_eq!(
        service_account(&settings),
        Some(r#"{"flag": true, "count": 2, "absent": null, "label": "a b", "nested": ["a", []]}"#)
    );
}

#[test]
fn explicit_credentials() {
    let environment = environment_of(None, None, Some("/env/sa.json"));
    let argument = Json::Text("argument-sa".to_owned());
    let deployment_wins = vertex_file_read_settings(
        &deployment_of(None, None, Some(text_of("deployment-sa"))),
        Some(&argument),
        &environment,
    );
    assert_eq!(service_account(&deployment_wins), Some("deployment-sa"));
    let argument_wins = vertex_file_read_settings(
        &deployment_of(None, None, None),
        Some(&argument),
        &environment,
    );
    assert_eq!(service_account(&argument_wins), Some("argument-sa"));
    let empty_deployment = vertex_file_read_settings(
        &deployment_of(None, None, Some(text_of(""))),
        Some(&argument),
        &environment,
    );
    assert_eq!(service_account(&empty_deployment), Some("argument-sa"));
}

#[test]
fn falsy_argument_credentials() {
    let environment = environment_of(None, None, Some("/env/sa.json"));
    let empty_dictionary_argument = Json::Object(IndexMap::new());
    let empty_dictionary = vertex_file_read_settings(
        &deployment_of(None, None, None),
        Some(&empty_dictionary_argument),
        &environment,
    );
    assert_eq!(service_account(&empty_dictionary), Some("{}"));
    let empty_text_argument = Json::Text(String::new());
    let empty_text = vertex_file_read_settings(
        &deployment_of(None, None, None),
        Some(&empty_text_argument),
        &environment,
    );
    assert_eq!(service_account(&empty_text), Some(""));
}

#[test]
fn service_account_fallback() {
    let environment = environment_of(None, None, Some("/env/sa.json"));
    let from_environment =
        vertex_file_read_settings(&deployment_of(None, None, None), None, &environment);
    assert_eq!(service_account(&from_environment), Some("/env/sa.json"));
    let unset = vertex_file_read_settings(
        &deployment_of(None, None, None),
        None,
        &environment_of(None, None, None),
    );
    assert_eq!(service_account(&unset), None);
    let empty = vertex_file_read_settings(
        &deployment_of(None, None, None),
        None,
        &environment_of(None, None, Some("")),
    );
    assert_eq!(service_account(&empty), None);
}

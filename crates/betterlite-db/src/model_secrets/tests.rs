#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredValue,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::{DEFAULT_USER_ID, ModelSecretsError, PreparedModel, prepare_encrypted_model};
use crate::stored_cipher::DbCipher;

const OLD_KEY: &str = "old-master-key";
const NEW_KEY: &str = "new-master-key";

const UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

fn cipher(key: &str) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn new_key() -> SecretString {
    SecretString::from(NEW_KEY)
}

fn open(text: &str, key: &str) -> Option<String> {
    let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    match cipher(key).decrypt_value(stored, "litellm_params", UNREADABLE) {
        Some(StoredValue::Text(plain)) => Some(plain.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => None,
    }
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn deployment() -> JsonObject {
    object(vec![
        ("model_name", text("gpt-4o")),
        (
            "litellm_params",
            Json::Object(object(vec![
                ("model", text("openai/gpt-4o")),
                ("api_key", Json::Null),
                ("api_base", text("https://example.test")),
                ("timeout", Json::Number(Number::from(30_i64))),
                ("stream", Json::Bool(true)),
            ])),
        ),
        (
            "model_info",
            Json::Object(object(vec![
                ("id", text("model-1")),
                ("db_model", Json::Bool(true)),
                ("created_by", Json::Null),
            ])),
        ),
    ])
}

fn prepare(record: &JsonObject, actor: Option<&str>) -> Result<PreparedModel, ModelSecretsError> {
    prepare_encrypted_model(record, &cipher(OLD_KEY), &new_key(), actor)
}

fn params(prepared: &PreparedModel) -> JsonObject {
    serde_json::from_str(&prepared.litellm_params).expect("the params text is a JSON object")
}

fn info(prepared: &PreparedModel) -> JsonObject {
    serde_json::from_str(&prepared.model_info).expect("the info text is a JSON object")
}

#[test]
fn prepared_model_seals_every_string_with_the_new_key() {
    let prepared = prepare(&deployment(), Some("user-1")).unwrap();
    let params = params(&prepared);
    let Json::Text(model) = &params["model"] else {
        panic!("the model parameter stays a string");
    };
    let Json::Text(api_base) = &params["api_base"] else {
        panic!("the api_base parameter stays a string");
    };
    assert_eq!(open(model, NEW_KEY).as_deref(), Some("openai/gpt-4o"));
    assert_eq!(
        open(api_base, NEW_KEY).as_deref(),
        Some("https://example.test")
    );
    assert!(open(model, OLD_KEY).is_none());
    assert!(open(api_base, OLD_KEY).is_none());
}

#[test]
fn prepared_model_keeps_non_string_parameters_and_drops_nulls() {
    let prepared = prepare(&deployment(), Some("user-1")).unwrap();
    let params = params(&prepared);
    assert_eq!(params["timeout"], Json::Number(Number::from(30_i64)));
    assert_eq!(params["stream"], Json::Bool(true));
    assert!(params.get("api_key").is_none());
}

#[test]
fn prepared_model_serializes_params_and_info_separately() {
    let prepared = prepare(&deployment(), Some("user-1")).unwrap();
    assert_eq!(prepared.model_id, "model-1");
    assert_eq!(prepared.model_name, "gpt-4o");
    let params = params(&prepared);
    let info = info(&prepared);
    assert!(params.get("db_model").is_none());
    assert_eq!(info["id"], text("model-1"));
    assert_eq!(info["db_model"], Json::Bool(true));
    assert!(info.get("created_by").is_none());
}

#[test]
fn prepared_model_uses_the_actor_for_both_audit_fields() {
    let prepared = prepare(&deployment(), Some("user-1")).unwrap();
    assert_eq!(prepared.created_by, "user-1");
    assert_eq!(prepared.updated_by, "user-1");
}

#[test]
fn prepared_model_falls_back_to_the_default_user_id() {
    let prepared = prepare(&deployment(), None).unwrap();
    assert_eq!(prepared.created_by, DEFAULT_USER_ID);
    assert_eq!(prepared.updated_by, DEFAULT_USER_ID);
}

#[test]
fn prepared_model_keeps_an_explicit_empty_actor() {
    let prepared = prepare(&deployment(), Some("")).unwrap();
    assert_eq!(prepared.created_by, "");
    assert_eq!(prepared.updated_by, "");
}

#[rstest]
#[case::missing_params("litellm_params")]
#[case::missing_model_info("model_info")]
fn a_record_without_an_object_field_is_rejected(#[case] name: &'static str) {
    let mut record = deployment();
    record.shift_remove(name);
    let error = prepare(&record, None).unwrap_err();
    assert!(matches!(error, ModelSecretsError::MissingObject { field } if field == name));
}

#[test]
fn a_record_whose_params_are_not_an_object_is_rejected() {
    let mut record = deployment();
    record.insert("litellm_params".to_owned(), text("openai/gpt-4o"));
    let error = prepare(&record, None).unwrap_err();
    assert!(matches!(
        error,
        ModelSecretsError::MissingObject {
            field: "litellm_params"
        }
    ));
}

#[test]
fn a_record_without_a_model_id_is_rejected() {
    let mut record = deployment();
    record.insert(
        "model_info".to_owned(),
        Json::Object(object(vec![("db_model", Json::Bool(true))])),
    );
    let error = prepare(&record, None).unwrap_err();
    assert!(matches!(
        error,
        ModelSecretsError::MissingText { field: "id" }
    ));
}

#[test]
fn a_record_without_a_model_name_is_rejected() {
    let mut record = deployment();
    record.insert("model_name".to_owned(), Json::Number(Number::from(1_i64)));
    let error = prepare(&record, None).unwrap_err();
    assert!(matches!(
        error,
        ModelSecretsError::MissingText {
            field: "model_name"
        }
    ));
}

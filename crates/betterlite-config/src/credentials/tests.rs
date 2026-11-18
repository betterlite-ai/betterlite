#![cfg(test)]

use std::collections::BTreeMap;
use std::mem::size_of;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;

use crate::credentials::{CredentialValue, CredentialValues};
use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::credential_list::Settings;

const VALID: &str = include_str!("../../fuzz/corpus/config_yaml/credentials-valid.yaml");

fn settings(source: &str) -> Result<Settings, ConfigError> {
    Ok(ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)?
    .sections
    .credential_list)
}

fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

fn number(value: i64) -> JsonData {
    JsonData::Number(serde_json::Number::from(value))
}

fn values(entries: &[(&str, JsonData)]) -> CredentialValues {
    let fields = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    CredentialValues::from(Payload(fields))
}

#[test]
fn loads_credential_list_settings_stay_behind_one_pointer() {
    assert_eq!(size_of::<Settings>(), size_of::<usize>());
}

#[test]
fn loads_credential_list_credential_values_compare_by_contents() {
    let text_left = values(&[("api_key", text("sk-one"))]);
    assert_eq!(text_left, values(&[("api_key", text("sk-one"))]));
    assert_ne!(text_left, values(&[("api_key", text("sk-two"))]));
    let port = number(443);
    let other_port = number(8443);
    let other_left = values(&[("port", port.clone())]);
    assert_eq!(other_left, values(&[("port", port)]));
    assert_ne!(other_left, values(&[("port", other_port)]));
    assert_ne!(text_left, other_left);
}

#[test]
fn loads_credential_list() {
    let configured = settings(VALID).unwrap();
    assert_eq!(configured.credentials.len(), 2);
    let openai = &configured.credentials[0];
    assert_eq!(openai.credential_name, "openai");
    assert_eq!(
        values(&[
            ("api_key", text("sk-config")),
            ("organization", text("org-1"))
        ]),
        openai.credential_values
    );
    assert_eq!(
        openai.credential_info.0.get("team"),
        Some(&JsonData::String("ml".to_owned()))
    );
    let azure = &configured.credentials[1];
    assert_eq!(azure.credential_name, "azure/prod");
    assert_eq!(azure.credential_info, Payload(BTreeMap::new()));
}

#[test]
fn loads_credential_list_a_missing_credential_list_keeps_the_default() {
    let configured = settings("model_list: []\n").unwrap();
    assert!(configured.credentials.is_empty());
}

#[test]
fn loads_credential_list_a_null_credential_list_keeps_the_default() {
    let configured = settings("credential_list: null\n").unwrap();
    assert!(configured.credentials.is_empty());
}

#[test]
fn loads_credential_list_an_empty_credential_list_stays_empty() {
    let configured = settings(include_str!(
        "../../fuzz/corpus/config_yaml/credentials-empty.yaml"
    ))
    .unwrap();
    assert!(configured.credentials.is_empty());
}

#[test]
fn loads_credential_list_a_credential_list_that_is_not_a_list_is_refused() {
    let error = settings("credential_list: not-a-list\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("expected sequence start"), "{error}");
}

#[test]
fn loads_credential_list_an_entry_without_values_is_refused() {
    let error = settings(include_str!(
        "../../fuzz/corpus/config_yaml/credentials-invalid.yaml"
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("credential_values"), "{error}");
}

#[test]
fn loads_credential_list_a_non_object_entry_is_refused() {
    let error = settings("credential_list:\n  - 42\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("expected mapping start"), "{error}");
}

#[test]
fn loads_credential_list_a_non_object_credential_value_is_refused() {
    let error = settings(
        "credential_list:\n  - credential_name: leaky\n    credential_info: {}\n    credential_values: not-a-mapping\n",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("credential_values must be a mapping of names to values"),
        "{error}"
    );
}

#[test]
fn loads_credential_list_debug_redacts_credential_values() {
    let configured = settings(VALID).unwrap();
    let printed = format!("{configured:?}");
    assert!(printed.contains("REDACTED"), "{printed}");
    assert!(!printed.contains("sk-config"), "{printed}");
    assert!(!printed.contains("sk-azure"), "{printed}");
}

#[test]
fn loads_credential_list_nested_values_keep_their_shape_and_hide_secrets() {
    let original = JsonData::Object(BTreeMap::from([
        ("token".to_owned(), text("nested-secret")),
        (
            "flags".to_owned(),
            JsonData::Array(vec![JsonData::Null, JsonData::Bool(false), number(3)]),
        ),
    ]));
    let protected = CredentialValue::from(original.clone());
    assert_eq!(protected.expose_json(), original);
    let printed = format!("{protected:?}");
    assert!(!printed.contains("nested-secret"));
    let CredentialValue::Object(fields) = protected else {
        panic!("nested objects retain their structure");
    };
    assert!(matches!(
        fields.get("token"),
        Some(CredentialValue::Text(_))
    ));
}

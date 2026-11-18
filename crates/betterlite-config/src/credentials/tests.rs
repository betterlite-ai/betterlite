#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;

use crate::loader::ConfigLoader;
use crate::sections::credential_list::Settings;

const VALID: &str = include_str!("../../fuzz/corpus/config_yaml/credentials-valid.yaml");

fn settings(source: &str) -> Result<Settings, String> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.credential_list)
    .map_err(|error| error.to_string())
}

#[test]
fn loads_credential_list() {
    let configured = settings(VALID).unwrap();
    assert_eq!(configured.credentials.len(), 2);
    let openai = &configured.credentials[0];
    assert_eq!(openai.fields.base.credential_name, "openai");
    assert_eq!(
        openai.fields.credential_values.0.get("api_key"),
        Some(&JsonData::String("sk-config".to_owned()))
    );
    assert_eq!(
        openai.fields.credential_values.0.get("organization"),
        Some(&JsonData::String("org-1".to_owned()))
    );
    assert_eq!(
        openai.fields.base.credential_info.0.get("team"),
        Some(&JsonData::String("ml".to_owned()))
    );
    let azure = &configured.credentials[1];
    assert_eq!(azure.fields.base.credential_name, "azure/prod");
    assert_eq!(azure.fields.base.credential_info, Payload(BTreeMap::new()));
}

#[test]
fn a_missing_credential_list_keeps_the_default() {
    let configured = settings("model_list: []\n").unwrap();
    assert!(configured.credentials.is_empty());
}

#[test]
fn a_null_credential_list_keeps_the_default() {
    let configured = settings("credential_list: null\n").unwrap();
    assert!(configured.credentials.is_empty());
}

#[test]
fn an_empty_credential_list_stays_empty() {
    let configured = settings(include_str!(
        "../../fuzz/corpus/config_yaml/credentials-empty.yaml"
    ))
    .unwrap();
    assert!(configured.credentials.is_empty());
}

#[test]
fn a_credential_list_that_is_not_a_list_is_refused() {
    let error = settings("credential_list: not-a-list\n").unwrap_err();
    assert!(error.contains("credential_list must be a list"), "{error}");
}

#[test]
fn an_entry_without_values_names_its_index() {
    let error = settings(include_str!(
        "../../fuzz/corpus/config_yaml/credentials-invalid.yaml"
    ))
    .unwrap_err();
    assert!(error.contains("credential_list[0]"), "{error}");
}

#[test]
fn a_non_object_entry_is_refused() {
    let error = settings("credential_list:\n  - 42\n").unwrap_err();
    assert!(error.contains("credential_list[0]"), "{error}");
}

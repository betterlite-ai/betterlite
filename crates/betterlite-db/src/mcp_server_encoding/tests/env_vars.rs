#![cfg(test)]

use betterlite_secrets::secret_codec::StoredValue;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::McpEncodingError;
use super::super::env_vars::{
    decrypt_global_env_var_values, decrypt_returned_row_env_vars, decrypt_stored_env_var_values,
    encrypt_global_env_var_values, is_global_env_var_scope,
};
use super::super::prepare::prepare_mcp_server_data;
use super::super::request::{PrepareMode, PreparedServerData};
use super::{
    create, decrypt_cipher, encrypt_cipher, entries, env_text, field_set, json, payload, request,
    row, typed_env_var, update,
};
use crate::managed_resource_rows::mcp::McpEnvVarScope;
use crate::mcp_stored_contracts::JsonValue;

fn sealed(text: &str) -> String {
    let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    match encrypt_cipher().encrypt_value(stored, None).unwrap() {
        StoredValue::Text(sealed) => sealed.expose_secret().to_owned(),
        StoredValue::Other(()) => panic!("a text value has to stay text"),
    }
}

fn stored_env_vars(prepared: &PreparedServerData) -> Vec<JsonValue> {
    let JsonValue::Array(items) = env_text(prepared, "env_vars") else {
        panic!("env_vars is not stored as a JSON list");
    };
    items
}

fn decrypted_env_vars(prepared: &PreparedServerData) -> Vec<JsonValue> {
    let mut items = stored_env_vars(prepared);
    decrypt_stored_env_var_values(&mut items, &decrypt_cipher());
    items
}

fn env_field(entry: &JsonValue, name: &str) -> Option<String> {
    let JsonValue::Object(fields) = entry else {
        panic!("the env var entry is not an object");
    };
    match fields.get(name) {
        Some(JsonValue::String(text)) => Some(text.clone()),
        Some(other) => panic!("the env var {name} is not a string: {other:?}"),
        None => None,
    }
}

fn env_value(entry: &JsonValue) -> String {
    env_field(entry, "value").unwrap_or_else(|| panic!("the env var entry carries no value"))
}

fn env_scope(entry: &JsonValue) -> Option<String> {
    env_field(entry, "scope")
}

#[rstest]
#[case::absent(None, true)]
#[case::global(Some(McpEnvVarScope::Global), true)]
#[case::user(Some(McpEnvVarScope::User), false)]
fn is_global_env_var_scope_excludes_only_user(
    #[case] scope: Option<McpEnvVarScope>,
    #[case] expected: bool,
) {
    assert_eq!(is_global_env_var_scope(scope), expected);
}

#[test]
fn encrypt_global_env_var_values_encrypts_global_and_default_scopes() {
    let cipher = encrypt_cipher();
    let mut env_vars = entries(
        r#"[{"name":"A","value":"secret"},{"name":"B","value":"placeholder","scope":"user"},{"name":"C","value":"other","scope":"global"}]"#,
    );
    encrypt_global_env_var_values(&mut env_vars, &cipher).unwrap();
    assert_ne!(env_value(&env_vars[0]), "secret");
    assert_eq!(env_value(&env_vars[1]), "placeholder");
    assert_ne!(env_value(&env_vars[2]), "other");
    decrypt_stored_env_var_values(&mut env_vars, &decrypt_cipher());
    assert_eq!(env_value(&env_vars[0]), "secret");
    assert_eq!(env_value(&env_vars[1]), "placeholder");
    assert_eq!(env_value(&env_vars[2]), "other");
}

#[test]
fn encrypt_global_env_var_values_skips_empty_values_and_missing_values() {
    let cipher = encrypt_cipher();
    let mut env_vars = entries(r#"[{"name":"A","value":""},{"name":"B"}]"#);
    encrypt_global_env_var_values(&mut env_vars, &cipher).unwrap();
    assert_eq!(env_value(&env_vars[0]), "");
    assert_eq!(env_vars[1], json(r#"{"name":"B"}"#));
}

#[test]
fn encrypt_global_env_var_values_leaves_a_non_string_value_alone() {
    let cipher = encrypt_cipher();
    let mut env_vars = entries(r#"[{"name":"A","value":5}]"#);
    encrypt_global_env_var_values(&mut env_vars, &cipher).unwrap();
    assert_eq!(env_vars[0], json(r#"{"name":"A","value":5}"#));
}

#[test]
fn encrypt_global_env_var_values_rejects_a_non_object_entry() {
    let cipher = encrypt_cipher();
    let mut env_vars = entries("[5]");
    assert!(matches!(
        encrypt_global_env_var_values(&mut env_vars, &cipher),
        Err(McpEncodingError::EnvVarNotObject)
    ));
}

#[test]
fn decrypt_global_env_var_values_decrypts_globals_and_keeps_user_placeholders() {
    let mut env_vars = vec![
        typed_env_var("A", &sealed("secret"), McpEnvVarScope::Global),
        typed_env_var("B", "placeholder", McpEnvVarScope::User),
        typed_env_var("C", "", McpEnvVarScope::Global),
    ];
    decrypt_global_env_var_values(&mut env_vars, &decrypt_cipher());
    assert_eq!(env_vars[0].value, "secret");
    assert_eq!(env_vars[1].value, "placeholder");
    assert_eq!(env_vars[2].value, "");
}

#[test]
fn decrypt_global_env_var_values_clears_a_value_that_does_not_decrypt() {
    let mut env_vars = vec![
        typed_env_var("A", "not-a-ciphertext", McpEnvVarScope::Global),
        typed_env_var("B", "not-a-ciphertext", McpEnvVarScope::User),
    ];
    decrypt_global_env_var_values(&mut env_vars, &decrypt_cipher());
    assert_eq!(env_vars[0].value, "");
    assert_eq!(env_vars[1].value, "not-a-ciphertext");
}

#[test]
fn decrypt_stored_env_var_values_treats_a_missing_or_unknown_scope_as_global() {
    let mut env_vars = entries(&format!(
        r#"[{{"name":"A","value":"{}"}},{{"name":"B","value":"{}","scope":"team"}},{{"name":"C","value":"{}","scope":"user"}}]"#,
        sealed("a-secret"),
        sealed("b-secret"),
        sealed("c-secret")
    ));
    decrypt_stored_env_var_values(&mut env_vars, &decrypt_cipher());
    assert_eq!(env_value(&env_vars[0]), "a-secret");
    assert_eq!(env_value(&env_vars[1]), "b-secret");
    assert_ne!(env_value(&env_vars[2]), "c-secret");
}

#[test]
fn decrypt_stored_env_var_values_clears_an_undecryptable_global_value() {
    let mut env_vars = entries(r#"[{"name":"A","value":"not-a-ciphertext","scope":"global"}]"#);
    decrypt_stored_env_var_values(&mut env_vars, &decrypt_cipher());
    assert_eq!(env_value(&env_vars[0]), "");
}

#[test]
fn decrypt_stored_env_var_values_skips_non_object_entries_and_non_string_values() {
    let mut env_vars = entries(r#"[5,{"name":"A","value":7},{"name":"B","value":""}]"#);
    decrypt_stored_env_var_values(&mut env_vars, &decrypt_cipher());
    assert_eq!(env_vars[0], json("5"));
    assert_eq!(env_vars[1], json(r#"{"name":"A","value":7}"#));
    assert_eq!(env_value(&env_vars[2]), "");
}

#[test]
fn decrypt_returned_row_env_vars_parses_a_json_string_back_into_a_list() {
    let mut returned = row(&format!(
        r#"{{"env_vars":"[{{\"name\":\"A\",\"value\":\"{}\",\"scope\":\"global\"}}]"}}"#,
        sealed("secret")
    ));
    decrypt_returned_row_env_vars(&mut returned, &decrypt_cipher()).unwrap();
    let Some(JsonValue::Array(items)) = returned.get("env_vars") else {
        panic!("the returned env_vars did not become a list");
    };
    assert_eq!(env_value(&items[0]), "secret");
}

#[test]
fn decrypt_returned_row_env_vars_decrypts_an_array_in_place() {
    let mut returned = row(&format!(
        r#"{{"env_vars":[{{"name":"A","value":"{}","scope":"global"}}]}}"#,
        sealed("secret")
    ));
    decrypt_returned_row_env_vars(&mut returned, &decrypt_cipher()).unwrap();
    let Some(JsonValue::Array(items)) = returned.get("env_vars") else {
        panic!("the returned env_vars is not a list");
    };
    assert_eq!(env_value(&items[0]), "secret");
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"env_vars":null}"#)]
#[case::invalid_json(r#"{"env_vars":"not json"}"#)]
#[case::json_object(r#"{"env_vars":"{\"a\":\"b\"}"}"#)]
#[case::object(r#"{"env_vars":{"a":"b"}}"#)]
fn decrypt_returned_row_env_vars_leaves_a_non_list_payload_alone(#[case] stored: &str) {
    let mut returned = row(stored);
    let original = returned.clone();
    decrypt_returned_row_env_vars(&mut returned, &decrypt_cipher()).unwrap();
    assert_eq!(returned, original);
}

#[test]
fn decrypt_returned_row_env_vars_rejects_a_number_payload() {
    let mut returned = row(r#"{"env_vars":7}"#);
    assert!(matches!(
        decrypt_returned_row_env_vars(&mut returned, &decrypt_cipher()),
        Err(McpEncodingError::EnvVarsShape)
    ));
}

#[test]
fn prepare_create_encrypts_global_env_vars_and_keeps_user_placeholders() {
    let prepared = create(&request(
        r#"{"server_name":"srv","env_vars":[{"name":"A","value":"secret"},{"name":"B","value":"placeholder","scope":"user"},{"name":"C","value":""}]}"#,
    ));
    let stored = stored_env_vars(&prepared);
    assert_ne!(env_value(&stored[0]), "secret");
    assert_eq!(env_value(&stored[1]), "placeholder");
    assert_eq!(env_value(&stored[2]), "");
    assert_eq!(env_scope(&stored[0]), Some("global".to_owned()));
    assert_eq!(env_scope(&stored[1]), Some("user".to_owned()));
    let decoded = decrypted_env_vars(&prepared);
    assert_eq!(env_value(&decoded[0]), "secret");
    assert_eq!(env_value(&decoded[1]), "placeholder");
}

#[test]
fn prepare_create_rejects_a_null_env_var_value() {
    let error = prepare_mcp_server_data(
        &request(r#"{"server_name":"srv","env_vars":[{"name":"A","value":null}]}"#),
        PrepareMode::Create,
        &encrypt_cipher(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        McpEncodingError::EnvVarNull { field: "value", .. }
    ));
}

#[test]
fn prepare_update_keeps_env_var_entry_field_presence() {
    let prepared = update(
        &request(
            r#"{"server_id":"s1","env_vars":[{"name":"A","value":"secret"},{"name":"B","value":"","scope":"user"}]}"#,
        ),
        &["server_id", "env_vars"],
        &["server_id", "env_vars"],
    );
    let expected = json(&format!(
        r#"[{{"name":"A","value":"{}"}},{{"name":"B","value":"","scope":"user"}}]"#,
        sealed("secret")
    ));
    assert_eq!(env_text(&prepared, "env_vars"), expected);
}

#[test]
fn prepare_update_keeps_an_explicit_null_env_vars() {
    let prepared = update(
        &request(r#"{"server_id":"s1","env_vars":null}"#),
        &["server_id", "env_vars"],
        &["server_id", "env_vars"],
    );
    assert_eq!(prepared.get("env_vars"), Some(&JsonValue::Null));
}

#[test]
fn prepare_update_drops_env_vars_the_caller_never_sent() {
    let prepared = update(
        &request(r#"{"server_id":"s1"}"#),
        &["server_id"],
        &["server_id"],
    );
    assert_eq!(prepared, payload(r#"{"server_id":"s1"}"#));
}

#[test]
fn prepare_create_drops_a_null_env_vars() {
    let prepared = create(&request(r#"{"server_name":"srv","env_vars":null}"#));
    assert!(!prepared.contains_key("env_vars"));
}

#[test]
fn prepare_create_defaults_a_missing_env_var_value_and_scope() {
    let prepared = create(&request(
        r#"{"server_name":"srv","env_vars":[{"name":"A"}]}"#,
    ));
    assert_eq!(
        env_text(&prepared, "env_vars"),
        json(r#"[{"name":"A","value":"","scope":"global"}]"#)
    );
}

#[test]
fn prepare_update_keeps_a_missing_env_var_value_and_scope_out_of_the_entry() {
    let prepared = update(
        &request(r#"{"server_id":"s1","env_vars":[{"name":"A"}]}"#),
        &["server_id", "env_vars"],
        &["server_id", "env_vars"],
    );
    assert_eq!(env_text(&prepared, "env_vars"), json(r#"[{"name":"A"}]"#));
}

#[test]
fn prepare_create_drops_a_null_env_var_description() {
    let prepared = create(&request(
        r#"{"server_name":"srv","env_vars":[{"name":"A","description":null}]}"#,
    ));
    assert_eq!(
        env_text(&prepared, "env_vars"),
        json(r#"[{"name":"A","value":"","scope":"global"}]"#)
    );
}

#[test]
fn prepare_update_keeps_an_explicit_null_env_var_description() {
    let prepared = update(
        &request(r#"{"server_id":"s1","env_vars":[{"name":"A","description":null}]}"#),
        &["server_id", "env_vars"],
        &["server_id", "env_vars"],
    );
    assert_eq!(
        env_text(&prepared, "env_vars"),
        json(r#"[{"name":"A","description":null}]"#)
    );
}

#[test]
fn prepare_update_rejects_a_null_env_var_scope() {
    let error = prepare_mcp_server_data(
        &request(r#"{"server_id":"s1","env_vars":[{"name":"A","scope":null}]}"#),
        PrepareMode::Update {
            model_fields: &field_set(&["server_id", "env_vars"]),
            original_fields: &field_set(&["server_id", "env_vars"]),
        },
        &encrypt_cipher(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        McpEncodingError::EnvVarNull { field: "scope", .. }
    ));
}

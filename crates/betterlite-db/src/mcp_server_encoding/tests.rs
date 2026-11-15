#![cfg(test)]

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::secret_map::decode_secret_map;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, NonceSource, OsNonces, StoredCipher,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::McpEncodingError;
use super::blank::blank_to_none;
use super::credentials::{
    CredentialBlob, CredentialClass, credential_auth_class, credentials_blob_to_dict,
    decrypt_credentials, drop_stale_minted_tokens, encrypt_credentials,
};
use super::env_vars::ReturnedRow;
use super::prepare::prepare_mcp_server_data;
use super::request::{FieldsSet, McpServerRequest, PrepareMode, PreparedServerData};
use crate::managed_resource_rows::mcp::{McpEnvVar, McpEnvVarScope};
use crate::mcp_stored_contracts::{JsonValue, MCPAuth};
use crate::stored_cipher::DbCipher;

mod env_vars;
mod warnings;

const SALT: &str = "test-salt";
const NONCE: [u8; 24] = [9; 24];

struct FixedNonces([u8; 24]);

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        nonce.copy_from_slice(&self.0[..nonce.len()]);
        Ok(())
    }
}

fn settings() -> CipherSettings {
    CipherSettings {
        signing_key: Some(SecretString::from(SALT)),
        algorithm: Algorithm::XSalsa20Poly1305,
    }
}

fn encrypt_cipher() -> StoredCipher<FixedNonces, TracingDiagnostics> {
    StoredCipher {
        settings: settings(),
        nonces: FixedNonces(NONCE),
        diagnostics: TracingDiagnostics,
    }
}

fn decrypt_cipher() -> DbCipher {
    StoredCipher {
        settings: settings(),
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn s(text: &str) -> JsonValue {
    JsonValue::String(text.to_owned())
}

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn payload(text: &str) -> PreparedServerData {
    let JsonValue::Object(fields) = json(text) else {
        panic!("the expected payload is not a JSON object");
    };
    fields
}

fn blob(text: &str) -> CredentialBlob {
    let JsonValue::Object(fields) = json(text) else {
        panic!("the expected credentials are not a JSON object");
    };
    fields
}

fn entries(text: &str) -> Vec<JsonValue> {
    let JsonValue::Array(items) = json(text) else {
        panic!("the expected env vars are not a JSON array");
    };
    items
}

fn field_set(names: &[&str]) -> FieldsSet {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn request(text: &str) -> McpServerRequest {
    serde_json::from_str(text).unwrap()
}

fn env_text(prepared: &PreparedServerData, field: &str) -> JsonValue {
    let Some(JsonValue::String(text)) = prepared.get(field) else {
        panic!("{field} is not stored as JSON text");
    };
    serde_json::from_str(text).unwrap()
}

fn decrypted_blob(prepared: &PreparedServerData) -> CredentialBlob {
    let Some(stored) = prepared.get("credentials") else {
        panic!("credentials are missing from the payload");
    };
    let mut decoded = credentials_blob_to_dict(stored).unwrap();
    decrypt_credentials(&mut decoded, &decrypt_cipher());
    decoded
}

fn decrypted_secret_map(prepared: &PreparedServerData, field: &str) -> JsonValue {
    let Some(JsonValue::String(text)) = prepared.get(field) else {
        panic!("{field} is not stored as JSON text");
    };
    let decoded = decode_secret_map(&decrypt_cipher(), Some(text.as_str()), field)
        .unwrap()
        .unwrap();
    JsonValue::Object(
        decoded
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    JsonValue::String(value.expose_secret().to_owned()),
                )
            })
            .collect(),
    )
}

fn create(request: &McpServerRequest) -> PreparedServerData {
    prepare_mcp_server_data(request, PrepareMode::Create, &encrypt_cipher()).unwrap()
}

fn update(request: &McpServerRequest, model: &[&str], original: &[&str]) -> PreparedServerData {
    let model_fields = field_set(model);
    let original_fields = field_set(original);
    prepare_mcp_server_data(
        request,
        PrepareMode::Update {
            model_fields: &model_fields,
            original_fields: &original_fields,
        },
        &encrypt_cipher(),
    )
    .unwrap()
}

fn row(text: &str) -> ReturnedRow {
    let JsonValue::Object(fields) = json(text) else {
        panic!("the expected row is not a JSON object");
    };
    fields
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), None)]
#[case::spaces(Some("   "), None)]
#[case::mixed_whitespace(Some(" \t\n\r"), None)]
#[case::surrounded(Some("  value  "), Some("value"))]
#[case::inner_space(Some("two words"), Some("two words"))]
#[case::zero_text(Some("0"), Some("0"))]
fn blank_to_none_trims_a_value_and_clears_a_blank(
    #[case] input: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(blank_to_none(input), expected);
}

#[test]
fn blank_to_none_has_no_non_string_input_at_the_typed_boundary() {
    assert!(serde_json::from_str::<McpServerRequest>(r#"{"issuer": 5}"#).is_err());
    assert!(serde_json::from_str::<McpServerRequest>(r#"{"audience": ["a"]}"#).is_err());
}

#[rstest]
#[case::absent(None, None)]
#[case::true_passthrough(Some(MCPAuth::TruePassthrough), Some(CredentialClass::ClientForwarded))]
#[case::oauth_delegate(Some(MCPAuth::OauthDelegate), Some(CredentialClass::ClientForwarded))]
#[case::oauth2(Some(MCPAuth::Oauth2), Some(CredentialClass::Auth(MCPAuth::Oauth2)))]
#[case::api_key(Some(MCPAuth::ApiKey), Some(CredentialClass::Auth(MCPAuth::ApiKey)))]
#[case::aws_sigv4(
    Some(MCPAuth::AwsSigv4),
    Some(CredentialClass::Auth(MCPAuth::AwsSigv4))
)]
fn credential_auth_class_collapses_only_the_client_forwarded_modes(
    #[case] auth_type: Option<MCPAuth>,
    #[case] expected: Option<CredentialClass>,
) {
    assert_eq!(credential_auth_class(auth_type), expected);
}

#[test]
fn credential_auth_class_keeps_a_switch_within_the_client_forwarded_class() {
    assert_eq!(
        credential_auth_class(Some(MCPAuth::TruePassthrough)),
        credential_auth_class(Some(MCPAuth::OauthDelegate))
    );
    assert_ne!(
        credential_auth_class(Some(MCPAuth::Oauth2)),
        credential_auth_class(Some(MCPAuth::OauthDelegate))
    );
}

#[rstest]
#[case::no_client_rotation(
    r#"{"client_id":"old","access_token":"stale","refresh_token":"stale","expires_in":"60"}"#,
    r#"{"aws_region_name":"us-east-1"}"#,
    r#"{"client_id":"old","access_token":"stale","refresh_token":"stale","expires_in":"60","aws_region_name":"us-east-1"}"#
)]
#[case::client_id_rotation(
    r#"{"client_id":"old","access_token":"stale","refresh_token":"stale","expires_in":"60","aws_region_name":"us-east-1"}"#,
    r#"{"client_id":"new"}"#,
    r#"{"client_id":"new","aws_region_name":"us-east-1"}"#
)]
#[case::client_secret_rotation(
    r#"{"client_id":"old","client_secret":"old-secret","access_token":"stale"}"#,
    r#"{"client_secret":"new-secret"}"#,
    r#"{"client_id":"old","client_secret":"new-secret"}"#
)]
#[case::rotation_resets_the_same_client(
    r#"{"client_id":"same","access_token":"stale"}"#,
    r#"{"client_id":"same"}"#,
    r#"{"client_id":"same"}"#
)]
#[case::new_minted_token_survives(
    r#"{"client_id":"old","access_token":"stale","refresh_token":"stale","expires_in":"60"}"#,
    r#"{"client_id":"new","access_token":"fresh"}"#,
    r#"{"client_id":"new","access_token":"fresh"}"#
)]
fn drop_stale_minted_tokens_follows_a_client_rotation(
    #[case] existing: &str,
    #[case] update: &str,
    #[case] expected: &str,
) {
    let mut merged = blob(existing);
    let new_credentials = blob(update);
    for (field, value) in &new_credentials {
        merged.insert(field.clone(), value.clone());
    }
    drop_stale_minted_tokens(&mut merged, &new_credentials);
    assert_eq!(merged, blob(expected));
}

fn typed_env_var(name: &str, value: &str, scope: McpEnvVarScope) -> McpEnvVar {
    McpEnvVar {
        name: name.to_owned(),
        value: value.to_owned(),
        scope,
        description: None,
    }
}

#[test]
fn encrypt_credentials_encrypts_the_secret_fields_only() {
    let cipher = encrypt_cipher();
    let plaintext = blob(
        r#"{"auth_value":"auth","client_id":"id","client_secret":"secret","client_private_key":"key","aws_access_key_id":"access","aws_secret_access_key":"aws-secret","aws_session_token":"session","aws_region_name":"us-east-1","scopes":["read"],"access_token":"minted"}"#,
    );
    let mut credentials = plaintext.clone();
    encrypt_credentials(&mut credentials, &cipher).unwrap();
    assert_ne!(
        credentials.get("auth_value"),
        plaintext.get("auth_value"),
        "auth_value was left in plaintext"
    );
    assert_ne!(
        credentials.get("client_id"),
        plaintext.get("client_id"),
        "client_id was left in plaintext"
    );
    assert_ne!(
        credentials.get("client_secret"),
        plaintext.get("client_secret"),
        "client_secret was left in plaintext"
    );
    assert_ne!(
        credentials.get("client_private_key"),
        plaintext.get("client_private_key"),
        "client_private_key was left in plaintext"
    );
    assert_eq!(credentials.get("aws_region_name"), Some(&s("us-east-1")));
    assert_eq!(credentials.get("access_token"), Some(&s("minted")));
    assert_eq!(credentials.get("scopes"), Some(&json(r#"["read"]"#)));
}

#[test]
fn encrypt_credentials_encrypts_the_aws_secret_fields_only() {
    let cipher = encrypt_cipher();
    let plaintext = blob(
        r#"{"auth_value":"auth","client_id":"id","client_secret":"secret","client_private_key":"key","aws_access_key_id":"access","aws_secret_access_key":"aws-secret","aws_session_token":"session","aws_region_name":"us-east-1","scopes":["read"],"access_token":"minted"}"#,
    );
    let mut credentials = plaintext.clone();
    encrypt_credentials(&mut credentials, &cipher).unwrap();
    assert_ne!(
        credentials.get("aws_access_key_id"),
        plaintext.get("aws_access_key_id"),
        "aws_access_key_id was left in plaintext"
    );
    assert_ne!(
        credentials.get("aws_secret_access_key"),
        plaintext.get("aws_secret_access_key"),
        "aws_secret_access_key was left in plaintext"
    );
    assert_ne!(
        credentials.get("aws_session_token"),
        plaintext.get("aws_session_token"),
        "aws_session_token was left in plaintext"
    );
    assert_eq!(credentials.get("aws_region_name"), Some(&s("us-east-1")));
}

#[test]
fn encrypt_credentials_leaves_null_and_non_string_values_alone() {
    let cipher = encrypt_cipher();
    let mut credentials = blob(r#"{"client_id":null,"client_secret":5,"scopes":["read"]}"#);
    let original = credentials.clone();
    encrypt_credentials(&mut credentials, &cipher).unwrap();
    assert_eq!(credentials, original);
}

#[test]
fn decrypt_credentials_restores_the_secret_fields() {
    let cipher = encrypt_cipher();
    let plaintext = blob(
        r#"{"auth_value":"auth","client_id":"id","client_secret":"secret","client_private_key":"key","aws_access_key_id":"access","aws_secret_access_key":"aws-secret","aws_session_token":"session","aws_region_name":"us-east-1"}"#,
    );
    let mut sealed_blob = plaintext.clone();
    encrypt_credentials(&mut sealed_blob, &cipher).unwrap();
    decrypt_credentials(&mut sealed_blob, &decrypt_cipher());
    assert_eq!(sealed_blob, plaintext);
}

#[test]
fn decrypt_credentials_forwards_a_value_it_cannot_decrypt() {
    let mut credentials = blob(r#"{"client_id":"not-a-ciphertext","aws_region_name":"us-east-1"}"#);
    decrypt_credentials(&mut credentials, &decrypt_cipher());
    assert_eq!(credentials.get("client_id"), Some(&s("not-a-ciphertext")));
    assert_eq!(credentials.get("aws_region_name"), Some(&s("us-east-1")));
}

#[rstest]
#[case::object(r#"{"client_id":"abc"}"#, true)]
#[case::json_text(r#""{\"client_id\":\"abc\"}""#, true)]
#[case::invalid_json(r#""not json""#, false)]
#[case::json_list(r#""[1,2]""#, false)]
#[case::number("5", false)]
#[case::null("null", false)]
#[case::array("[1]", false)]
fn credentials_blob_to_dict_reads_a_string_or_an_object(
    #[case] stored: &str,
    #[case] expected_ok: bool,
) {
    let result = credentials_blob_to_dict(&json(stored));
    assert_eq!(result.is_ok(), expected_ok);
    if let Ok(decoded) = result {
        assert_eq!(decoded.get("client_id"), Some(&s("abc")));
    }
}

#[test]
fn credentials_blob_to_dict_reports_a_parse_error() {
    assert!(matches!(
        credentials_blob_to_dict(&s("not json")),
        Err(McpEncodingError::Json(_))
    ));
    assert!(matches!(
        credentials_blob_to_dict(&s("[1]")),
        Err(McpEncodingError::CredentialsNotObject)
    ));
}

#[test]
fn prepare_create_writes_litellm_defaults_and_forces_is_byok() {
    let prepared = create(&request(r#"{"server_name":"srv"}"#));
    let expected = payload(
        r#"{"alias":null,"allow_all_keys":false,"args":[],"available_on_public_internet":true,"byok_description":[],"delegate_auth_to_upstream":false,"env":"{}","is_byok":false,"mcp_access_groups":[],"oauth_passthrough":false,"per_server_oauth_discovery":false,"server_name":"srv","transport":"sse"}"#,
    );
    assert_eq!(prepared, expected);
}

#[test]
fn prepare_create_drops_nulls_and_keeps_explicit_values() {
    let prepared = create(&request(
        r#"{"server_id":"s1","server_name":"srv","alias":"alias","description":"d","transport":"http","auth_type":"oauth2","url":"https://up","spec_path":"/spec","instructions":"use","command":"run","args":["a"],"mcp_access_groups":["g"],"allowed_tools":["t"],"extra_headers":["h"],"issuer":"https://issuer","authorization_url":"https://auth","token_url":"https://token","registration_url":"https://register","oauth2_flow":"authorization_code","dcr_bridge":true,"allow_all_keys":true,"available_on_public_internet":false,"delegate_auth_to_upstream":true,"oauth_passthrough":true,"per_server_oauth_discovery":true,"is_byok":true,"byok_description":["k"],"byok_api_key_help_url":"https://help","source_url":"https://source","max_concurrent_requests":3}"#,
    ));
    assert_eq!(prepared.get("server_id"), Some(&s("s1")));
    assert_eq!(prepared.get("alias"), Some(&s("alias")));
    assert_eq!(prepared.get("transport"), Some(&s("http")));
    assert_eq!(prepared.get("auth_type"), Some(&s("oauth2")));
    assert_eq!(prepared.get("oauth2_flow"), Some(&s("authorization_code")));
    assert_eq!(prepared.get("dcr_bridge"), Some(&JsonValue::Bool(true)));
    assert_eq!(prepared.get("allow_all_keys"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        prepared.get("available_on_public_internet"),
        Some(&JsonValue::Bool(false))
    );
    assert_eq!(prepared.get("is_byok"), Some(&JsonValue::Bool(true)));
    assert_eq!(prepared.get("args"), Some(&json(r#"["a"]"#)));
    assert_eq!(prepared.get("mcp_access_groups"), Some(&json(r#"["g"]"#)));
    assert_eq!(prepared.get("max_concurrent_requests"), Some(&json("3")));
    assert!(!prepared.contains_key("credentials"));
    assert!(!prepared.contains_key("mcp_info"));
    assert!(!prepared.contains_key("static_headers"));
    assert!(!prepared.contains_key("env_vars"));
}

#[test]
fn prepare_create_writes_a_float_timeout_as_a_number() {
    let prepared = create(&request(r#"{"server_name":"srv","timeout":30}"#));
    assert_eq!(prepared.get("timeout"), Some(&json("30.0")));
}

#[test]
fn prepare_create_encrypts_credentials_and_drops_null_values() {
    let prepared = create(&request(
        r#"{"server_id":"s1","server_name":"srv","credentials":{"client_id":"abc","client_secret":null,"aws_region_name":"us-east-1"}}"#,
    ));
    let decoded = decrypted_blob(&prepared);
    assert_eq!(decoded.get("client_id"), Some(&s("abc")));
    assert_eq!(decoded.get("aws_region_name"), Some(&s("us-east-1")));
    assert!(!decoded.contains_key("client_secret"));
}

// LiteLLM's pydantic model drops a key outside MCPCredentials, and serde drops it the same way.
#[test]
fn prepare_create_drops_a_credential_field_outside_the_typed_model() {
    let prepared = create(&request(
        r#"{"server_name":"srv","credentials":{"client_id":"abc","future_field":5}}"#,
    ));
    let decoded = decrypted_blob(&prepared);
    assert_eq!(decoded.get("client_id"), Some(&s("abc")));
    assert!(!decoded.contains_key("future_field"));
}

#[test]
fn prepare_create_lifts_token_exchange_settings_out_of_the_credentials_blob() {
    let prepared = create(&request(
        r#"{"server_name":"srv","credentials":{"client_id":"abc","token_exchange_endpoint":"https://idp/token","audience":"aud","subject_token_type":"urn:type","token_exchange_profile":"profile"}}"#,
    ));
    assert_eq!(
        prepared.get("token_exchange_endpoint"),
        Some(&s("https://idp/token"))
    );
    assert_eq!(prepared.get("audience"), Some(&s("aud")));
    assert_eq!(prepared.get("subject_token_type"), Some(&s("urn:type")));
    assert_eq!(prepared.get("token_exchange_profile"), Some(&s("profile")));
    let decoded = decrypted_blob(&prepared);
    assert!(!decoded.contains_key("token_exchange_endpoint"));
    assert!(!decoded.contains_key("audience"));
    assert!(!decoded.contains_key("subject_token_type"));
    assert!(!decoded.contains_key("token_exchange_profile"));
    assert_eq!(decoded.get("client_id"), Some(&s("abc")));
}

#[test]
fn prepare_create_keeps_an_explicit_top_level_token_exchange_value() {
    let prepared = create(&request(
        r#"{"server_name":"srv","token_exchange_endpoint":"https://top","credentials":{"token_exchange_endpoint":"https://blob"}}"#,
    ));
    assert_eq!(
        prepared.get("token_exchange_endpoint"),
        Some(&s("https://top"))
    );
    let decoded = decrypted_blob(&prepared);
    assert!(!decoded.contains_key("token_exchange_endpoint"));
}

#[test]
fn prepare_create_lifts_a_null_token_exchange_blob_value_nowhere() {
    let prepared = create(&request(
        r#"{"server_name":"srv","credentials":{"token_exchange_endpoint":null}}"#,
    ));
    assert!(!prepared.contains_key("token_exchange_endpoint"));
    assert_eq!(decrypted_blob(&prepared), CredentialBlob::new());
}

#[test]
fn prepare_create_encrypts_static_headers_and_the_stdio_env_map() {
    let prepared = create(&request(
        r#"{"server_name":"srv","static_headers":{"X":"y"},"env":{"E":"1"}}"#,
    ));
    assert_eq!(
        decrypted_secret_map(&prepared, "static_headers"),
        json(r#"{"X":"y"}"#)
    );
    assert_eq!(decrypted_secret_map(&prepared, "env"), json(r#"{"E":"1"}"#));
}

#[test]
fn prepare_create_stores_an_empty_secret_map_as_an_empty_object() {
    let prepared = create(&request(r#"{"server_name":"srv","static_headers":{}}"#));
    assert_eq!(prepared.get("static_headers"), Some(&s("{}")));
    assert_eq!(prepared.get("env"), Some(&s("{}")));
}

#[test]
fn prepare_create_serializes_mcp_info_and_tool_maps_as_json_text() {
    let prepared = create(&request(
        r#"{"server_name":"srv","mcp_info":{"protocol_version":"auto","custom":{"keep":null}},"tool_name_to_display_name":{"t":"Tool"},"tool_name_to_description":{"t":"Desc"}}"#,
    ));
    assert_eq!(
        env_text(&prepared, "mcp_info"),
        json(r#"{"protocol_version":"auto","custom":{"keep":null}}"#)
    );
    assert_eq!(
        env_text(&prepared, "tool_name_to_display_name"),
        json(r#"{"t":"Tool"}"#)
    );
    assert_eq!(
        env_text(&prepared, "tool_name_to_description"),
        json(r#"{"t":"Desc"}"#)
    );
}

#[test]
fn prepare_create_keeps_a_nul_byte_that_safe_dumps_would_strip() {
    let prepared = create(&request(
        r#"{"server_name":"srv","mcp_info":{"note":"a\u0000b"}}"#,
    ));
    assert_eq!(
        env_text(&prepared, "mcp_info"),
        json(r#"{"note":"a\u0000b"}"#)
    );
}

#[test]
fn prepare_create_drops_null_json_map_fields_and_allowed_tools() {
    let prepared = create(&request(
        r#"{"server_name":"srv","tool_name_to_display_name":null,"tool_name_to_description":null,"mcp_info":null,"allowed_tools":null}"#,
    ));
    assert!(!prepared.contains_key("tool_name_to_display_name"));
    assert!(!prepared.contains_key("tool_name_to_description"));
    assert!(!prepared.contains_key("mcp_info"));
    assert!(!prepared.contains_key("allowed_tools"));
}

#[test]
fn prepare_update_writes_only_the_model_fields() {
    let prepared = update(
        &request(r#"{"server_id":"s1","server_name":"srv"}"#),
        &["server_id", "server_name"],
        &["server_id", "server_name"],
    );
    assert_eq!(
        prepared,
        payload(r#"{"server_id":"s1","server_name":"srv"}"#)
    );
}

#[test]
fn prepare_update_keeps_an_omitted_field_out_of_the_payload() {
    let prepared = update(
        &request(r#"{"server_id":"s1","transport":"stdio","allow_all_keys":true}"#),
        &["server_id"],
        &["server_id"],
    );
    assert_eq!(prepared, payload(r#"{"server_id":"s1"}"#));
}

#[test]
fn prepare_update_drops_an_assigned_null_alias_the_caller_never_sent() {
    let prepared = update(
        &request(r#"{"server_id":"s1"}"#),
        &["server_id", "alias"],
        &["server_id"],
    );
    assert_eq!(prepared, payload(r#"{"server_id":"s1"}"#));
}

#[test]
fn prepare_update_keeps_an_explicit_null_alias_the_caller_sent() {
    let prepared = update(
        &request(r#"{"server_id":"s1","alias":null}"#),
        &["server_id", "alias"],
        &["server_id", "alias"],
    );
    assert_eq!(prepared, payload(r#"{"server_id":"s1","alias":null}"#));
}

#[test]
fn prepare_update_keeps_a_normalized_alias_the_body_omitted() {
    let prepared = update(
        &request(r#"{"server_id":"s1","server_name":"srv","alias":"srv"}"#),
        &["server_id", "server_name", "alias"],
        &["server_id", "server_name"],
    );
    assert_eq!(prepared.get("alias"), Some(&s("srv")));
}

#[test]
fn prepare_update_turns_a_null_allowed_tools_into_an_empty_list() {
    let prepared = update(
        &request(r#"{"server_id":"s1","allowed_tools":null}"#),
        &["server_id", "allowed_tools"],
        &["server_id", "allowed_tools"],
    );
    assert_eq!(
        prepared.get("allowed_tools"),
        Some(&JsonValue::Array(Vec::new()))
    );
}

#[test]
fn prepare_update_turns_null_json_maps_into_empty_objects() {
    let prepared = update(
        &request(
            r#"{"server_id":"s1","tool_name_to_display_name":null,"tool_name_to_description":null}"#,
        ),
        &[
            "server_id",
            "tool_name_to_display_name",
            "tool_name_to_description",
        ],
        &[
            "server_id",
            "tool_name_to_display_name",
            "tool_name_to_description",
        ],
    );
    assert_eq!(prepared.get("tool_name_to_display_name"), Some(&s("{}")));
    assert_eq!(prepared.get("tool_name_to_description"), Some(&s("{}")));
}

#[test]
fn prepare_update_keeps_an_explicit_null_mcp_info() {
    let prepared = update(
        &request(r#"{"server_id":"s1","mcp_info":null}"#),
        &["server_id", "mcp_info"],
        &["server_id", "mcp_info"],
    );
    assert_eq!(prepared.get("mcp_info"), Some(&JsonValue::Null));
}

#[test]
fn prepare_update_keeps_an_explicit_null_static_headers() {
    let prepared = update(
        &request(r#"{"server_id":"s1","static_headers":null}"#),
        &["server_id", "static_headers"],
        &["server_id", "static_headers"],
    );
    assert_eq!(prepared.get("static_headers"), Some(&JsonValue::Null));
}

#[test]
fn prepare_update_encrypts_credentials_without_dropping_null_values() {
    let prepared = update(
        &request(r#"{"server_id":"s1","credentials":{"client_id":"abc","client_secret":null}}"#),
        &["server_id", "credentials"],
        &["server_id", "credentials"],
    );
    let decoded = decrypted_blob(&prepared);
    assert_eq!(decoded.get("client_id"), Some(&s("abc")));
    assert_eq!(decoded.get("client_secret"), Some(&JsonValue::Null));
}

#[test]
fn prepare_update_writes_is_byok_only_when_the_caller_sent_it() {
    let omitted = update(
        &request(r#"{"server_id":"s1"}"#),
        &["server_id"],
        &["server_id"],
    );
    assert!(!omitted.contains_key("is_byok"));
    let explicit = update(
        &request(r#"{"server_id":"s1","is_byok":false}"#),
        &["server_id", "is_byok"],
        &["server_id", "is_byok"],
    );
    assert_eq!(explicit.get("is_byok"), Some(&JsonValue::Bool(false)));
}

#[test]
fn prepare_rejects_an_unknown_request_field() {
    assert!(
        serde_json::from_str::<McpServerRequest>(r#"{"server_name":"srv","future":1}"#).is_err()
    );
}

#[test]
fn prepare_rejects_an_unknown_env_var_scope() {
    assert!(
        serde_json::from_str::<McpServerRequest>(
            r#"{"server_name":"srv","env_vars":[{"name":"A","scope":"team"}]}"#
        )
        .is_err()
    );
}

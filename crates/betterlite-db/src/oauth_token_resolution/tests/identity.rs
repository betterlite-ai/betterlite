use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::managed_resource_rows::mcp::{McpOAuth2Flow, McpServerTable};
use crate::mcp_server_encoding::credentials::CredentialBlob;
use crate::mcp_stored_contracts::{JsonValue, MCPAuth, MCPCredentials};
use crate::oauth_token_resolution::identity::{DecryptedField, decrypted_credential_field};
use crate::oauth_token_resolution::mcp_oauth_token_identity;
use crate::stored_cipher::DbCipher;

use super::support::{cipher, exposed, sealed};

const CLIENT_ID: &str = "client-one";
const CLIENT_SECRET: &str = "secret-one";

fn server(credentials: &str) -> McpServerTable {
    serde_json::from_str(&format!(
        r#"{{"server_id":"s1","transport":"sse","url":"https://mcp.example/sse","spec_path":"/specs/s1.json","auth_type":"oauth2","oauth2_flow":"authorization_code","issuer":"https://issuer.example","authorization_url":"https://issuer.example/authorize","token_url":"https://issuer.example/token","registration_url":"https://issuer.example/register","credentials":{credentials}}}"#
    ))
    .expect("the test server is valid")
}

fn credentials(text: &str) -> MCPCredentials {
    serde_json::from_str(text).expect("the test credentials are valid")
}

fn credential_json(client_id: &str, client_secret: &str, scopes: &str) -> String {
    format!(
        r#"{{"client_id":"{client_id}","client_secret":"{client_secret}","scopes":{scopes},"upstream_resource":"https://api.example"}}"#
    )
}

fn encrypted_credentials(cipher: &DbCipher) -> String {
    credential_json(
        &sealed(cipher, CLIENT_ID),
        &sealed(cipher, CLIENT_SECRET),
        r#"["read","write"]"#,
    )
}

fn field(name: &str, value: JsonValue) -> CredentialBlob {
    CredentialBlob::from([(name.to_owned(), value)])
}

#[test]
fn decrypted_credential_field_returns_an_absent_field_as_null() {
    let blob = CredentialBlob::new();
    let decrypted = decrypted_credential_field(&blob, "client_id", &cipher());
    assert!(matches!(decrypted, DecryptedField::Value(JsonValue::Null)));
}

#[test]
fn decrypted_credential_field_keeps_a_non_string_value() {
    let stored = JsonValue::Array(vec![JsonValue::String("value".to_owned())]);
    let blob = field("client_id", stored.clone());
    let decrypted = decrypted_credential_field(&blob, "client_id", &cipher());
    assert!(matches!(decrypted, DecryptedField::Value(value) if value == stored));
}

#[test]
fn decrypted_credential_field_decrypts_a_stored_ciphertext() {
    let cipher = cipher();
    let blob = field("client_id", JsonValue::String(sealed(&cipher, CLIENT_ID)));
    let decrypted = decrypted_credential_field(&blob, "client_id", &cipher);
    assert!(
        matches!(&decrypted, DecryptedField::Text(secret) if secret.expose_secret() == CLIENT_ID)
    );
}

#[test]
fn decrypted_credential_field_keeps_a_legacy_plaintext_value() {
    let blob = field("client_id", JsonValue::String("plain-client".to_owned()));
    let decrypted = decrypted_credential_field(&blob, "client_id", &cipher());
    assert!(
        matches!(&decrypted, DecryptedField::Text(secret) if secret.expose_secret() == "plain-client")
    );
}

#[test]
fn decrypted_credential_field_keeps_a_tampered_ciphertext() {
    let cipher = cipher();
    let tampered = format!("{}AAAA", sealed(&cipher, CLIENT_ID));
    let blob = field("client_id", JsonValue::String(tampered.clone()));
    let decrypted = decrypted_credential_field(&blob, "client_id", &cipher);
    assert!(
        matches!(&decrypted, DecryptedField::Text(secret) if secret.expose_secret() == tampered)
    );
}

#[test]
fn decrypted_field_debug_output_redacts_the_plaintext() {
    let decrypted = DecryptedField::Text(SecretString::from("top-secret"));
    let debug = format!("{decrypted:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("top-secret"));
}

#[test]
fn identity_reads_the_server_fields() {
    let cipher = cipher();
    let identity = mcp_oauth_token_identity(&server(&encrypted_credentials(&cipher)), &cipher);
    assert_eq!(identity.url.as_deref(), Some("https://mcp.example/sse"));
    assert_eq!(identity.spec_path.as_deref(), Some("/specs/s1.json"));
    assert_eq!(identity.auth_type, Some(MCPAuth::Oauth2));
    assert_eq!(identity.oauth2_flow, Some(McpOAuth2Flow::AuthorizationCode));
    assert_eq!(identity.issuer.as_deref(), Some("https://issuer.example"));
    assert_eq!(
        identity.authorization_url.as_deref(),
        Some("https://issuer.example/authorize")
    );
    assert_eq!(
        identity.token_url.as_deref(),
        Some("https://issuer.example/token")
    );
    assert_eq!(
        identity.registration_url.as_deref(),
        Some("https://issuer.example/register")
    );
}

#[test]
fn identity_decrypts_the_client_credentials() {
    let cipher = cipher();
    let identity = mcp_oauth_token_identity(&server(&encrypted_credentials(&cipher)), &cipher);
    assert_eq!(identity.client_id.as_deref(), Some(CLIENT_ID));
    assert_eq!(
        exposed(identity.client_secret.as_ref()),
        Some(CLIENT_SECRET)
    );
    assert_eq!(
        identity.scopes,
        Some(vec!["read".to_owned(), "write".to_owned()])
    );
    assert_eq!(
        identity.upstream_resource.as_deref(),
        Some("https://api.example")
    );
}

#[test]
fn identity_compares_client_credentials_after_decryption() {
    let cipher = cipher();
    let first = mcp_oauth_token_identity(&server(&encrypted_credentials(&cipher)), &cipher);
    let second = mcp_oauth_token_identity(&server(&encrypted_credentials(&cipher)), &cipher);
    assert_eq!(first, second);
}

fn change(server: &mut McpServerTable, field: &str, cipher: &DbCipher) {
    match field {
        "url" | "spec_path" | "auth_type" | "oauth2_flow" | "issuer" | "authorization_url"
        | "token_url" | "registration_url" => change_server_field(server, field),
        other => change_credential_field(server, other, cipher),
    }
}

fn change_server_field(server: &mut McpServerTable, field: &str) {
    match field {
        "url" => server.url = Some("https://other.example/sse".to_owned()),
        "spec_path" => server.spec_path = Some("/specs/other.json".to_owned()),
        "auth_type" => server.auth_type = Some(MCPAuth::BearerToken),
        "oauth2_flow" => server.oauth2_flow = Some(McpOAuth2Flow::ClientCredentials),
        "issuer" => server.issuer = Some("https://other.example".to_owned()),
        "authorization_url" => {
            server.authorization_url = Some("https://other.example/authorize".to_owned());
        }
        "token_url" => server.token_url = Some("https://other.example/token".to_owned()),
        "registration_url" => {
            server.registration_url = Some("https://other.example/register".to_owned());
        }
        other => panic!("unknown identity server field {other}"),
    }
}

fn change_credential_field(server: &mut McpServerTable, field: &str, cipher: &DbCipher) {
    let client_id = sealed(cipher, CLIENT_ID);
    let client_secret = sealed(cipher, CLIENT_SECRET);
    let replaced = match field {
        "client_id" => credential_json(
            &sealed(cipher, "client-two"),
            &client_secret,
            r#"["read","write"]"#,
        ),
        "client_secret" => credential_json(
            &client_id,
            &sealed(cipher, "secret-two"),
            r#"["read","write"]"#,
        ),
        "scopes" => credential_json(&client_id, &client_secret, r#"["admin"]"#),
        "upstream_resource" => format!(
            r#"{{"client_id":"{client_id}","client_secret":"{client_secret}","scopes":["read","write"],"upstream_resource":"https://other.example"}}"#
        ),
        other => panic!("unknown identity field {other}"),
    };
    server.credentials = Some(credentials(&replaced));
}

#[rstest]
#[case::url("url")]
#[case::spec_path("spec_path")]
#[case::auth_type("auth_type")]
#[case::oauth2_flow("oauth2_flow")]
#[case::issuer("issuer")]
#[case::authorization_url("authorization_url")]
#[case::token_url("token_url")]
#[case::registration_url("registration_url")]
#[case::client_id("client_id")]
#[case::client_secret("client_secret")]
#[case::scopes("scopes")]
#[case::upstream_resource("upstream_resource")]
fn identity_changes_when_one_field_changes(#[case] field: &str) {
    let cipher = cipher();
    let base = mcp_oauth_token_identity(&server(&encrypted_credentials(&cipher)), &cipher);
    let mut changed = server(&encrypted_credentials(&cipher));
    change(&mut changed, field, &cipher);
    assert_ne!(base, mcp_oauth_token_identity(&changed, &cipher));
}

use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};

use crate::managed_resource_rows::mcp::{McpOAuth2Flow, McpServerTable};
use crate::mcp_server_encoding::credentials::CredentialBlob;
use crate::mcp_stored_contracts::{JsonValue, MCPAuth};
use crate::stored_cipher::DbCipher;

const DECRYPT_FIELD: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};

// LiteLLM's _decrypted_credential_field: a non-string stays as stored, and a string that is not ciphertext comes back unchanged.
#[must_use]
pub fn decrypted_credential_field(
    creds: &CredentialBlob,
    field: &str,
    cipher: &DbCipher,
) -> JsonValue {
    let Some(value) = creds.get(field) else {
        return JsonValue::Null;
    };
    let JsonValue::String(text) = value else {
        return value.clone();
    };
    let stored = StoredValue::<()>::Text(SecretString::from(text.clone()));
    let Some(StoredValue::Text(plaintext)) = cipher.decrypt_value(stored, field, DECRYPT_FIELD)
    else {
        return value.clone();
    };
    JsonValue::String(plaintext.expose_secret().to_owned())
}

// LiteLLM's tuple of the server fields that decide which upstream token is minted; a change purges the stored per-user tokens.
#[derive(Debug, Clone)]
pub struct McpOauthTokenIdentity {
    pub url: Option<String>,
    pub spec_path: Option<String>,
    pub auth_type: Option<MCPAuth>,
    pub oauth2_flow: Option<McpOAuth2Flow>,
    pub issuer: Option<String>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub registration_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub scopes: Option<Vec<String>>,
    pub upstream_resource: Option<String>,
}

impl PartialEq for McpOauthTokenIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.spec_path == other.spec_path
            && self.auth_type == other.auth_type
            && self.oauth2_flow == other.oauth2_flow
            && self.issuer == other.issuer
            && self.authorization_url == other.authorization_url
            && self.token_url == other.token_url
            && self.registration_url == other.registration_url
            && self.client_id == other.client_id
            && same_secret(&self.client_secret, &other.client_secret)
            && self.scopes == other.scopes
            && self.upstream_resource == other.upstream_resource
    }
}

// A fresh nonce makes the stored ciphertext differ on every write, so the decrypted secret is the compared value.
fn same_secret(left: &Option<SecretString>, right: &Option<SecretString>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => bool::from(
            left.expose_secret()
                .as_bytes()
                .ct_eq(right.expose_secret().as_bytes()),
        ),
        (Some(_), None) | (None, Some(_)) => false,
        (None, None) => true,
    }
}

// The credential fields identity reads, still as stored: client_id and client_secret keep their ciphertext until compared.
pub(super) fn credential_blob(server: &McpServerTable) -> CredentialBlob {
    let mut blob = CredentialBlob::new();
    let Some(credentials) = &server.credentials else {
        return blob;
    };
    if let Some(Some(client_id)) = &credentials.client_id {
        drop(blob.insert("client_id".to_owned(), JsonValue::String(client_id.clone())));
    }
    if let Some(Some(secret)) = &credentials.client_secret {
        let sealed = secret.0.expose_secret().to_owned();
        drop(blob.insert("client_secret".to_owned(), JsonValue::String(sealed)));
    }
    blob
}

// The decrypted value of a credential field that must be text to matter; absent, null and a non-string all compare as no value.
pub(super) fn decrypted_text(
    creds: &CredentialBlob,
    field: &str,
    cipher: &DbCipher,
) -> Option<String> {
    match decrypted_credential_field(creds, field, cipher) {
        JsonValue::String(text) => Some(text),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

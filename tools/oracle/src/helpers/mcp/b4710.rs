use betterlite_db::mcp_server_encoding::McpEncodingError;
use betterlite_db::mcp_server_encoding::credentials::{
    CredentialBlob, credentials_blob_to_dict, decrypt_credentials, encrypt_credentials,
};
use betterlite_db::mcp_server_encoding::env_vars::decrypt_stored_env_var_values;
use betterlite_db::mcp_server_encoding::prepare::prepare_mcp_server_data;
use betterlite_db::mcp_server_encoding::request::{
    FieldsSet, McpServerRequest, PrepareMode, PreparedServerData,
};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::secret_map::{SecretMap, decode_secret_map};
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, NonceSource, OsNonces, StoredCipher,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

const NONCE_BYTES: usize = 24;
const DEFAULT_NONCE: [u8; NONCE_BYTES] = [0; NONCE_BYTES];
const JSON_TEXT_FIELDS: [&str; 7] = [
    "credentials",
    "env",
    "env_vars",
    "mcp_info",
    "static_headers",
    "tool_name_to_description",
    "tool_name_to_display_name",
];

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    PrepareCreate {
        salt_key: String,
        request: McpServerRequest,
    },
    PrepareUpdate {
        salt_key: String,
        request: McpServerRequest,
        model_fields: Vec<String>,
        original_fields: Vec<String>,
    },
    EncryptCredentials {
        salt_key: String,
        credentials: CredentialBlob,
        nonce_hex: String,
    },
    DecryptCredentials {
        salt_key: String,
        credentials: CredentialBlob,
    },
    DecryptEnvVars {
        salt_key: String,
        env_vars: Vec<JsonValue>,
    },
}

pub(crate) struct Op;

struct FixedNonces([u8; NONCE_BYTES]);

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = self.0.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}

fn settings(salt: &str) -> CipherSettings {
    CipherSettings {
        signing_key: Some(SecretString::from(salt.to_owned())),
        algorithm: Algorithm::XSalsa20Poly1305,
    }
}

fn encrypt_cipher(
    salt: &str,
    nonce: [u8; NONCE_BYTES],
) -> StoredCipher<FixedNonces, TracingDiagnostics> {
    StoredCipher {
        settings: settings(salt),
        nonces: FixedNonces(nonce),
        diagnostics: TracingDiagnostics,
    }
}

fn decrypt_cipher(salt: &str) -> DbCipher {
    StoredCipher {
        settings: settings(salt),
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn encoding_error(error: McpEncodingError) -> Box<HelperError> {
    helper_error("McpEncodingError", &error.to_string())
}

fn output(value: JsonValue) -> Result<Value, Box<HelperError>> {
    serde_json::to_value(value)
        .map_err(|source| helper_error("SerializationError", &source.to_string()))
}

fn fixed_nonce(text: &str) -> Result<[u8; NONCE_BYTES], Box<HelperError>> {
    let bytes =
        hex::decode(text).map_err(|source| helper_error("InvalidNonce", &source.to_string()))?;
    <[u8; NONCE_BYTES]>::try_from(bytes).map_err(|_too_short| {
        helper_error("InvalidNonce", "the nonce must be 24 bytes of hexadecimal")
    })
}

fn secret_map_value(map: SecretMap) -> JsonValue {
    JsonValue::Object(
        map.into_iter()
            .map(|(name, secret)| (name, JsonValue::String(secret.expose_secret().to_owned())))
            .collect(),
    )
}

fn plaintext_value(
    field: &str,
    text: &str,
    cipher: &DbCipher,
) -> Result<JsonValue, McpEncodingError> {
    match field {
        "credentials" => {
            let mut credentials = credentials_blob_to_dict(&JsonValue::String(text.to_owned()))?;
            decrypt_credentials(&mut credentials, cipher);
            Ok(JsonValue::Object(credentials))
        }
        "static_headers" | "env" => {
            let decoded = decode_secret_map(cipher, Some(text), field)?;
            Ok(decoded.map_or(JsonValue::Null, secret_map_value))
        }
        "env_vars" => {
            let parsed: JsonValue = serde_json::from_str(text)?;
            let JsonValue::Array(mut entries) = parsed else {
                return Ok(parsed);
            };
            decrypt_stored_env_var_values(&mut entries, cipher);
            Ok(JsonValue::Array(entries))
        }
        _ => serde_json::from_str(text).map_err(McpEncodingError::Json),
    }
}

fn plaintext_view(
    mut data: PreparedServerData,
    cipher: &DbCipher,
) -> Result<JsonValue, McpEncodingError> {
    for field in JSON_TEXT_FIELDS {
        let stored = match data.get(field) {
            Some(JsonValue::String(text)) => text.clone(),
            Some(JsonValue::Null)
            | Some(JsonValue::Bool(_))
            | Some(JsonValue::Number(_))
            | Some(JsonValue::Array(_))
            | Some(JsonValue::Object(_))
            | None => continue,
        };
        let view = plaintext_value(field, &stored, cipher)?;
        data.insert(field.to_owned(), view);
    }
    Ok(JsonValue::Object(data))
}

fn prepare_create(salt: &str, request: &McpServerRequest) -> Result<Value, Box<HelperError>> {
    let prepared = prepare_mcp_server_data(
        request,
        PrepareMode::Create,
        &encrypt_cipher(salt, DEFAULT_NONCE),
    )
    .map_err(encoding_error)?;
    let view = plaintext_view(prepared, &decrypt_cipher(salt)).map_err(encoding_error)?;
    output(view)
}

fn prepare_update(
    salt: &str,
    request: &McpServerRequest,
    model_fields: &[String],
    original_fields: &[String],
) -> Result<Value, Box<HelperError>> {
    let model: FieldsSet = model_fields.iter().cloned().collect();
    let original: FieldsSet = original_fields.iter().cloned().collect();
    let prepared = prepare_mcp_server_data(
        request,
        PrepareMode::Update {
            model_fields: &model,
            original_fields: &original,
        },
        &encrypt_cipher(salt, DEFAULT_NONCE),
    )
    .map_err(encoding_error)?;
    let view = plaintext_view(prepared, &decrypt_cipher(salt)).map_err(encoding_error)?;
    output(view)
}

fn encrypt_step(
    salt: &str,
    credentials: &CredentialBlob,
    nonce_hex: &str,
) -> Result<Value, Box<HelperError>> {
    let mut encrypted = credentials.clone();
    let nonce = fixed_nonce(nonce_hex)?;
    encrypt_credentials(&mut encrypted, &encrypt_cipher(salt, nonce)).map_err(encoding_error)?;
    output(JsonValue::Object(encrypted))
}

fn decrypt_step(salt: &str, credentials: &CredentialBlob) -> Result<Value, Box<HelperError>> {
    let mut decrypted = credentials.clone();
    decrypt_credentials(&mut decrypted, &decrypt_cipher(salt));
    output(JsonValue::Object(decrypted))
}

fn decrypt_env_step(salt: &str, env_vars: &[JsonValue]) -> Result<Value, Box<HelperError>> {
    let mut decrypted = env_vars.to_vec();
    decrypt_stored_env_var_values(&mut decrypted, &decrypt_cipher(salt));
    output(JsonValue::Array(decrypted))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::PrepareCreate { salt_key, request } => prepare_create(salt_key, request),
            Input::PrepareUpdate {
                salt_key,
                request,
                model_fields,
                original_fields,
            } => prepare_update(salt_key, request, model_fields, original_fields),
            Input::EncryptCredentials {
                salt_key,
                credentials,
                nonce_hex,
            } => encrypt_step(salt_key, credentials, nonce_hex),
            Input::DecryptCredentials {
                salt_key,
                credentials,
            } => decrypt_step(salt_key, credentials),
            Input::DecryptEnvVars { salt_key, env_vars } => decrypt_env_step(salt_key, env_vars),
        }
    }
}

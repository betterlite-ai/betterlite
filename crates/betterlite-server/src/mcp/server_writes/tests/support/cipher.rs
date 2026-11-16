use betterlite_db::mcp_server_encoding::credentials::{
    CredentialBlob, credentials_blob_to_dict, decrypt_credentials, encrypt_credentials,
};
use betterlite_db::mcp_server_encoding::env_vars::decrypt_stored_env_var_values;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use secrecy::SecretString;

pub(crate) const SALT: &str = "mcp-write-salt";

pub(crate) fn cipher(salt: &str) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

// The stored credentials column holds the encrypted blob as JSON text.
pub(crate) fn seeded_credentials(fields: &[(&str, &str)], cipher: &DbCipher) -> String {
    let mut blob = CredentialBlob::new();
    for (name, value) in fields {
        blob.insert((*name).to_owned(), JsonValue::String((*value).to_owned()));
    }
    encrypt_credentials(&mut blob, cipher).expect("the seed credentials encrypt");
    serde_json::to_string(&JsonValue::Object(blob)).expect("the seed blob serializes")
}

pub(crate) fn opened_credentials(column: &str, cipher: &DbCipher) -> CredentialBlob {
    let stored: JsonValue = serde_json::from_str(column).expect("the stored credentials are JSON");
    let mut blob = credentials_blob_to_dict(&stored).expect("the stored credentials are an object");
    decrypt_credentials(&mut blob, cipher);
    blob
}

pub(crate) fn opened_env_vars(column: &str, cipher: &DbCipher) -> Vec<JsonValue> {
    let stored: JsonValue = serde_json::from_str(column).expect("the stored env vars are JSON");
    let inner: JsonValue = match stored {
        JsonValue::String(text) => {
            serde_json::from_str(&text).expect("the stored env vars text is JSON")
        }
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => other,
    };
    let JsonValue::Array(mut entries) = inner else {
        panic!("the stored env vars are an array");
    };
    decrypt_stored_env_var_values(&mut entries, cipher);
    entries
}

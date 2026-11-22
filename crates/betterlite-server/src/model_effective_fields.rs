use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};

pub mod credential;
pub mod effective;
pub mod error;
pub mod ptu;
pub mod rate_limits;
pub mod stored;
pub mod strategy;
mod tests;

// decrypt_value_helper with exception_type="debug" and return_original_value=True: a value that does not open stays as stored
#[must_use]
pub fn decrypted_text(cipher: &DbCipher, stored: &str, field: &str) -> String {
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Original,
    };
    let sealed = StoredValue::<()>::Text(SecretString::from(stored.to_owned()));
    match cipher.decrypt_value(sealed, field, policy) {
        Some(StoredValue::Text(opened)) => opened.expose_secret().to_owned(),
        Some(StoredValue::Other(())) | None => stored.to_owned(),
    }
}

// LiteLLM's _decrypted_model: a stored model that is not text names no model
#[must_use]
pub fn decrypted_model(cipher: &DbCipher, stored_model: Option<&Json>) -> Option<String> {
    match stored_model {
        Some(Json::Text(stored)) => Some(decrypted_text(cipher, stored, "model")),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

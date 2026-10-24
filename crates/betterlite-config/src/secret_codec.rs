use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use secrecy::SecretString;

use crate::loader::EnvironmentLookup;
use crate::raw::RawAlgorithm;
use crate::types::GeneralSettings;

// LiteLLM falls back to the legacy cipher for any other value, without a warning, so nothing here is rejected
pub(crate) fn general_settings(
    setting: Option<RawAlgorithm>,
    master_key: Option<SecretString>,
    lookup: &dyn EnvironmentLookup,
) -> GeneralSettings {
    let algorithm = match setting {
        Some(RawAlgorithm::Text(name)) if name.to_lowercase() == "aes-256-gcm" => {
            Algorithm::Aes256Gcm
        }
        Some(RawAlgorithm::Text(_) | RawAlgorithm::Other(_)) | None => Algorithm::XSalsa20Poly1305,
    };
    // an empty LITELLM_SALT_KEY is still the key; only an unset one falls back to the master key
    let signing_key = lookup
        .lookup("LITELLM_SALT_KEY")
        .map(SecretString::from)
        .or_else(|| master_key.clone());
    GeneralSettings {
        cipher: CipherSettings {
            signing_key,
            algorithm,
        },
        master_key,
    }
}

mod tests;

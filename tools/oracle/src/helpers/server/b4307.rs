use std::collections::BTreeMap;

use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use betterlite_server::cache_settings_read::stored::stored_from_raw;
use betterlite_server::cache_settings_read::view::effective_values;
use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    stored: Value,
    environment: BTreeMap<String, String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        // A case sets no salt key, so every stored value keeps its stored form, as get_secret without a key does.
        let cipher = DbCipher {
            settings: CipherSettings {
                signing_key: None,
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        };
        let stored = stored_from_raw(&input.stored, &cipher).or_value_error()?;
        let current = effective_values(&stored, &input.environment);
        Ok(Value::Object(current))
    }
}

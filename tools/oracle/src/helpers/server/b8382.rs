use std::collections::BTreeMap;

use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use betterlite_server::db_row_decrypt::env::decrypt_and_export_env;
use betterlite_server::db_row_decrypt::model_info::model_info_for;
use betterlite_server::db_row_decrypt::warnings::Warnings;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ModelInfo {
        rows: Vec<ProxyModelTable>,
    },
    ExportEnv {
        salt_key: String,
        values: Map<String, Value>,
        keep_original: bool,
        environment: BTreeMap<String, String>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ModelInfo { rows } => {
                let warnings = Warnings::default();
                let infos: Vec<_> = rows
                    .iter()
                    .map(|row| model_info_for(row, false, &warnings))
                    .collect();
                Ok(serde_json::to_value(infos)?)
            }
            Input::ExportEnv {
                salt_key,
                values,
                keep_original,
                environment,
            } => {
                let mut exported_into = environment.clone();
                let exported = decrypt_and_export_env(
                    values,
                    *keep_original,
                    &cipher(salt_key),
                    &mut exported_into,
                    &Warnings::default(),
                );
                let read_back: Map<String, Value> = values
                    .keys()
                    .map(|key| {
                        let value = exported_into
                            .get(key)
                            .map_or(Value::Null, |text| Value::String(text.clone()));
                        (key.clone(), value)
                    })
                    .collect();
                Ok(json!({"exported": exported, "environment": read_back}))
            }
        }
    }
}

fn cipher(salt_key: &str) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt_key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

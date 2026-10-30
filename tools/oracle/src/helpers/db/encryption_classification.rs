use betterlite_db::encryption_scan::{
    _classify_callback_value, _classify_secret_map, DbCipher, LocationReport, MigrationJson,
    classify_value,
};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub values: Vec<MigrationJson>,
    #[serde(default)]
    pub callbacks: Vec<MigrationJson>,
    #[serde(default = "default_key")]
    pub key: String,
}

fn default_key() -> String {
    let key = String::from("scan");
    key
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let cipher = DbCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from("scan-key")),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        };
        let mut report = LocationReport {
            location: String::from("values"),
            scanned: 0,
            migrated: 0,
            already_v2: 0,
            plaintext: 0,
            undecryptable: 0,
            legacy: 0,
        };
        for value in &input.values {
            let class = if matches!(value, MigrationJson::Object(_)) {
                _classify_secret_map(value, &cipher, &input.key)
            } else {
                Some(classify_value(value, &cipher, &input.key))
            };
            report.count(class).map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: "OverflowError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        }
        for value in &input.callbacks {
            report
                .count(Some(_classify_callback_value(value, &cipher)))
                .map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "OverflowError".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                })?;
        }
        Ok(serde_json::json!({"counts": report.as_dict()}))
    }
}

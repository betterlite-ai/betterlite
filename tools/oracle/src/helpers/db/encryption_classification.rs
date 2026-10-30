use betterlite_db::encryption_scan::report::{LocationReport, ValueClass};
use betterlite_db::encryption_scan::{
    DbCipher, MigrationJson, classify_callback_value, classify_secret_map, classify_value,
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
    #[serde(default)]
    pub key: String,
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
        let key = if input.key.is_empty() {
            "scan"
        } else {
            &input.key
        };
        for value in &input.values {
            let class = if matches!(value, MigrationJson::Object(_)) {
                classify_secret_map(value, &cipher, key)
            } else {
                Some(classify_value(value, &cipher, key))
            };
            count_value(&mut report, class)?;
        }
        for value in &input.callbacks {
            count_value(&mut report, Some(classify_callback_value(value, &cipher)))?;
        }
        Ok(serde_json::json!({"counts": report.as_dict()}))
    }
}

fn count_value(
    report: &mut LocationReport,
    class: Option<ValueClass>,
) -> Result<(), Box<crate::helpers::HelperError>> {
    match report.count(class) {
        Ok(()) => Ok(()),
        Err(error) => Err(Box::new(crate::helpers::HelperError {
            class: "OverflowError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}

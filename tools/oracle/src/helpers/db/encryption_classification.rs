use betterlite_db::encryption_scan::report::{LocationReport, MigrationReport};
use betterlite_db::encryption_scan::secret_text::SecretText;
use betterlite_db::encryption_scan::{
    DbCipher, MigrationJson, ScanError, classify_callback_value, classify_secret_map,
    classify_value, iter_callback_var_dicts,
};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use serde::{Deserialize, Serialize};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub salt_key: SecretText,
    pub values: Vec<MigrationJson>,
    #[serde(default)]
    pub callbacks: Vec<MigrationJson>,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub maps: Vec<MigrationJson>,
    pub metadata: Option<MigrationJson>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let cipher = DbCipher {
            settings: CipherSettings {
                signing_key: Some(input.salt_key.0.clone()),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        };
        let report = classify_input(input, &cipher)?;
        let counts = report.as_dict();
        let aggregate = MigrationReport {
            locations: vec![report],
        };
        let projection = aggregate.as_dict().map_err(|error| helper_error(&error))?;
        Ok(serde_json::json!({"counts": counts, "report": projection}))
    }
}

fn classify_input(
    input: &Input,
    cipher: &DbCipher,
) -> Result<LocationReport, Box<crate::helpers::HelperError>> {
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
        let class = Some(classify_value(value, cipher, key));
        report.count(class).map_err(|error| helper_error(&error))?;
    }
    for value in &input.maps {
        let class = classify_secret_map(value, cipher, key);
        report.count(class).map_err(|error| helper_error(&error))?;
    }
    let metadata_values = input
        .metadata
        .iter()
        .flat_map(iter_callback_var_dicts)
        .flat_map(|vars| vars.values());
    for value in input.callbacks.iter().chain(metadata_values) {
        let class = Some(classify_callback_value(value, cipher));
        report.count(class).map_err(|error| helper_error(&error))?;
    }
    Ok(report)
}

fn helper_error(error: &ScanError) -> Box<crate::helpers::HelperError> {
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "OverflowError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

use betterlite_db::encryption_scan::report::{LocationReport, MigrationReport, ValueClass};
use betterlite_db::encryption_scan::{
    _iter_callback_var_dicts, DbCipher, MigrationJson, ScanError, classify_callback_value,
    classify_secret_map, classify_value,
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
    #[serde(default)]
    pub maps: Vec<MigrationJson>,
    pub metadata: Option<MigrationJson>,
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
        let report = classify_input(input, &cipher)?;
        let counts = report.as_dict();
        let aggregate = MigrationReport {
            locations: vec![report],
        };
        let projection = aggregate.as_dict().map_err(helper_error)?;
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
        count_value(&mut report, Some(classify_value(value, cipher, key)))?;
    }
    for value in &input.maps {
        count_value(&mut report, classify_secret_map(value, cipher, key))?;
    }
    let metadata_values = input
        .metadata
        .iter()
        .flat_map(_iter_callback_var_dicts)
        .flat_map(|vars| vars.values());
    for value in input.callbacks.iter().chain(metadata_values) {
        count_value(&mut report, Some(classify_callback_value(value, cipher)))?;
    }
    Ok(report)
}

fn count_value(
    report: &mut LocationReport,
    class: Option<ValueClass>,
) -> Result<(), Box<crate::helpers::HelperError>> {
    match report.count(class) {
        Ok(()) => Ok(()),
        Err(error) => Err(helper_error(error)),
    }
}

fn helper_error(error: ScanError) -> Box<crate::helpers::HelperError> {
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

use std::collections::BTreeSet;

use betterlite_redact::structured::{
    Payload, SensitiveDataMasker, mask_credentials_in_payload, mask_sensitive_structure,
    parse::parse_payload,
    redact::{mask_sensitive_keys, redact_credentials_in_payload},
};

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MaskSensitiveStructure {
        #[serde(default)]
        data: Payload,
    },
    MaskCredentialsInPayload {
        #[serde(default)]
        data: Payload,
    },
    RedactCredentialsInPayload {
        #[serde(default)]
        data: Payload,
        #[serde(default)]
        data_json: Option<String>,
    },
    MaskSensitiveKeys {
        #[serde(default)]
        data: Payload,
        sensitive_fields: BTreeSet<String>,
    },
    Mask {
        #[serde(default)]
        data: Payload,
    },
    MaskValue {
        data: String,
    },
    IsSensitiveKey {
        data: String,
        #[serde(default)]
        excluded_keys: Vec<String>,
    },
    MaskDict {
        #[serde(default)]
        data: Payload,
        #[serde(default)]
        excluded_keys: Vec<String>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let masker = SensitiveDataMasker::default();
        let result = match input {
            Input::MaskSensitiveStructure { data } => mask_sensitive_structure(data),
            Input::MaskCredentialsInPayload { data } => mask_credentials_in_payload(data),
            Input::RedactCredentialsInPayload { data, data_json } => {
                if data_json.is_some() && !data.is_null() {
                    return Err(invalid_mapping());
                }
                let decoded = data_json
                    .as_ref()
                    .map(|json| parse_payload(json.as_bytes()))
                    .transpose()
                    .map_err(|_error| invalid_mapping())?;
                let payload = decoded.as_ref().unwrap_or(data);
                redact_credentials_in_payload(payload.as_object().ok_or_else(invalid_mapping)?)
            }
            Input::MaskSensitiveKeys {
                data,
                sensitive_fields,
            } => mask_sensitive_keys(
                data.as_object().ok_or_else(invalid_mapping)?,
                sensitive_fields,
            ),
            Input::Mask { data } => masker.mask(data),
            Input::MaskValue { data } => Payload::String(masker.mask_value(data)),
            Input::IsSensitiveKey {
                data,
                excluded_keys,
            } => Payload::Bool(masker.is_sensitive_key(data, excluded_keys)),
            Input::MaskDict {
                data,
                excluded_keys,
            } => masker.mask_dict(
                data.as_object().ok_or_else(invalid_mapping)?,
                0,
                10,
                excluded_keys,
            ),
        };
        Ok(serde_json::json!({"result": result}))
    }
}

fn invalid_mapping() -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: "structured payload must be an object".to_owned(),
    };
    Box::new(error)
}

mod tests;

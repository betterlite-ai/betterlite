use std::collections::BTreeSet;

use betterlite_redact::structured::{
    Payload, SensitiveDataMasker, mask_credentials_in_payload, mask_sensitive_structure,
    redact::{mask_sensitive_keys, redact_credentials_in_payload},
};

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MaskSensitiveStructure {
        data: Payload,
    },
    MaskCredentialsInPayload {
        data: Payload,
    },
    RedactCredentialsInPayload {
        data: Payload,
    },
    MaskSensitiveKeys {
        data: Payload,
        sensitive_fields: BTreeSet<String>,
    },
    Mask {
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
        data: Payload,
        #[serde(default)]
        excluded_keys: Vec<String>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let masker = SensitiveDataMasker::new();
        let result = match input {
            Input::MaskSensitiveStructure { data } => mask_sensitive_structure(data),
            Input::MaskCredentialsInPayload { data } => mask_credentials_in_payload(data),
            Input::RedactCredentialsInPayload { data } => {
                redact_credentials_in_payload(data.as_object().ok_or_else(invalid_mapping)?)
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
        Ok(result)
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

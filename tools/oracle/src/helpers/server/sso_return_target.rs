use betterlite_server::sso_return_targets::SsoReturnTargetError;
use betterlite_server::sso_return_targets::origin::validate_return_to;
use betterlite_server::sso_return_targets::return_path::is_same_origin_return_path;
use serde::{Deserialize, Serialize};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    same_origin: Vec<String>,
    validate: Vec<ValidateCall>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidateCall {
    return_to: String,
    control_plane_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Validation {
    valid: Option<bool>,
    status: Option<u16>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let same_origin: Vec<bool> = input
            .same_origin
            .iter()
            .map(|target| is_same_origin_return_path(target))
            .collect();
        let mut validate: Vec<Validation> = Vec::with_capacity(input.validate.len());
        for call in &input.validate {
            let outcome = validate_return_to(&call.return_to, call.control_plane_url.as_deref());
            let validation = match outcome {
                Ok(valid) => Validation {
                    valid: Some(valid),
                    status: None,
                },
                Err(SsoReturnTargetError::OriginMismatch) => Validation {
                    valid: None,
                    status: Some(400),
                },
                Err(error) => return Err(error).or_value_error(),
            };
            validate.push(validation);
        }
        Ok(serde_json::json!({
            "same_origin": same_origin,
            "validate": validate,
        }))
    }
}

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use serde::Serialize;

use crate::lasso::error::LassoError;
use crate::lasso::wire::LassoResponse;
use crate::third_party_hooks_a::error::ThirdPartyHookError;

const VIOLATED_POLICY: &str = "Violated Lasso guardrail policy";

pub(super) fn api_error(error: LassoError) -> ThirdPartyHookError {
    match error {
        LassoError::Blocked { deputies, response } => blocked(deputies, &response),
        LassoError::Timeout { .. } => ThirdPartyHookError::Failure {
            message: "Lasso API timeout".to_owned(),
        },
        LassoError::InvalidApiKey => ThirdPartyHookError::Failure {
            message: "Invalid API key".to_owned(),
        },
        LassoError::RateLimited => ThirdPartyHookError::Failure {
            message: "Lasso API rate limit exceeded".to_owned(),
        },
        LassoError::HttpStatus { status } => ThirdPartyHookError::Failure {
            message: format!("API error: {}", status.as_u16()),
        },
        error @ (LassoError::InvalidHeader { .. }
        | LassoError::Request { .. }
        | LassoError::ResponseTooLarge { .. }
        | LassoError::ResponseJson { .. }) => ThirdPartyHookError::Failure {
            message: format!("Failed to verify request safety with Lasso API: {error}"),
        },
    }
}

#[derive(Serialize)]
struct BlockedDetail<'response> {
    error: &'static str,
    detection_message: String,
    lasso_response: &'response LassoResponse,
}

fn blocked(deputies: Vec<String>, response: &LassoResponse) -> ThirdPartyHookError {
    let detail = BlockedDetail {
        error: VIOLATED_POLICY,
        detection_message: format!("Blocking violations detected: {}", deputies.join(", ")),
        lasso_response: response,
    };
    let encoded =
        match serde_json::to_value(&detail).and_then(serde_json::from_value::<GuardrailValue>) {
            Ok(encoded) => encoded,
            Err(error) => {
                return ThirdPartyHookError::Failure {
                    message: format!("the Lasso response could not be encoded: {error}"),
                };
            }
        };
    ThirdPartyHookError::Rejection {
        status: StatusCode::BAD_REQUEST,
        detail: encoded,
        openai_code: None,
    }
}

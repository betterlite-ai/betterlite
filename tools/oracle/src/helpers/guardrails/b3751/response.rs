use betterlite_config::sections::akto_mirroring::UnreachableFallback;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::akto::error::AktoError;
use betterlite_guardrails::akto::response::AktoAnswer;
use reqwest::StatusCode;
use serde_json::json;

use super::Input;
use super::support::failure;

pub(super) fn init(input: &Input) -> crate::helpers::HelperResult {
    let guardrail = super::settings::build(input)?;
    Ok(json!({
        "base_url": guardrail.base_url,
        "account_id": guardrail.account_id,
        "vxlan_id": guardrail.vxlan_id,
        "unreachable_fallback": fallback_name(guardrail.unreachable_fallback),
        "timeout": guardrail.timeout.as_secs(),
    }))
}

fn fallback_name(fallback: UnreachableFallback) -> &'static str {
    match fallback {
        UnreachableFallback::FailClosed => "fail_closed",
        UnreachableFallback::FailOpen => "fail_open",
    }
}

pub(super) fn parse(input: &Input) -> crate::helpers::HelperResult {
    let guardrail = super::settings::build(input)?;
    let status = StatusCode::from_u16(input.answer.status.unwrap_or(200))
        .map_err(|error| failure("AktoError", None, error.to_string()))?;
    let body = answer_body(input)?;
    let verdict = guardrail
        .handle_guardrail_response(&AktoAnswer { status, body })
        .map_err(|error| failure("AktoError", None, error.to_string()))?;
    Ok(json!({"allowed": verdict.allowed, "reason": verdict.reason}))
}

fn answer_body(input: &Input) -> Result<String, Box<crate::helpers::HelperError>> {
    if let Some(raw) = &input.answer.raw_body {
        return Ok(raw.clone());
    }
    match &input.answer.body {
        Some(body) => serde_json::to_string(body)
            .map_err(|error| failure("JsonError", None, error.to_string())),
        None => Ok(String::new()),
    }
}

pub(super) fn unreachable(input: &Input) -> crate::helpers::HelperResult {
    let guardrail = super::settings::build(input)?;
    let error = malformed_response_error()?;
    match guardrail.handle_unreachable(super::akto_inputs(input), &error) {
        Ok(_inputs) => Ok(json!({"passthrough": super::inputs_json(input)})),
        Err(error) => Err(failure(
            "HTTPException",
            Some(error.status_code().as_u16()),
            error.to_string(),
        )),
    }
}

fn malformed_response_error() -> Result<AktoError, Box<crate::helpers::HelperError>> {
    let source = serde_json::from_str::<GuardrailValue>("not json")
        .err()
        .ok_or_else(|| {
            failure(
                "AktoError",
                None,
                "the malformed response parsed".to_owned(),
            )
        })?;
    Ok(AktoError::ResponseJson { source })
}

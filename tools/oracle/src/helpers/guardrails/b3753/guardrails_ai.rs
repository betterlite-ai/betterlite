use std::collections::BTreeMap;

use betterlite_guardrails::guardrails_ai_guardrail::error::GuardrailsAiGuardrailError;
use betterlite_guardrails::guardrails_ai_guardrail::{
    GuardrailsAiGuardrail, InputFormat, Settings as GuardrailsAiSettings,
};
use serde_json::json;

use super::request::GuardrailsAiProcessRequest;
use super::{OracleBlockable, fail, finish, prepare};
use crate::helpers::HelperResult;
use crate::mock::MockProvider;

pub(super) fn guardrails_ai_construct(
    guard_name: Option<String>,
    api_base: Option<String>,
    environment: &BTreeMap<String, String>,
) -> HelperResult {
    let settings =
        GuardrailsAiSettings::resolve(guard_name, api_base, InputFormat::LlmOutput, environment)
            .map_err(|error| fail(error.classify(), error.to_string()))?;
    Ok(json!({
        "guard_name": settings.guard_name,
        "api_base": settings.api_base,
        "input_format": settings.input_format,
    }))
}

pub(super) fn guardrails_ai_process_input(request: &GuardrailsAiProcessRequest) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(request.answer.as_ref())?;
    let guardrail = GuardrailsAiGuardrail {
        settings: GuardrailsAiSettings {
            guard_name: request.guard_name.clone(),
            api_base: base,
            input_format: request.input_format,
        },
        client: reqwest::Client::new(),
        timeout: std::time::Duration::from_secs(30),
    };
    let result = runtime.block_on(guardrail.process_input(
        request.last_user_message.clone(),
        &request.dynamic_params,
        &request.call_type,
    ));
    drop(silent);
    finish_guardrails_ai(mock, result)
}

impl OracleBlockable for GuardrailsAiGuardrailError {
    fn blocked_response(&self) -> Option<serde_json::Value> {
        match self {
            Self::Blocked { response } => Some(json!(response)),
            Self::MissingGuardName
            | Self::InvalidApiBase { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::PreCallBlocked { .. }
            | Self::PreCallMissingOutput
            | Self::InvalidUtf8 { .. } => None,
        }
    }

    fn status_u16(&self) -> u16 {
        self.status_code().as_u16()
    }

    fn classify(&self) -> &'static str {
        match self {
            Self::MissingGuardName => "Exception",
            Self::Blocked { .. } | Self::PreCallBlocked { .. } => "HTTPException",
            Self::InvalidApiBase { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::PreCallMissingOutput
            | Self::InvalidUtf8 { .. } => "GuardrailsAIException",
        }
    }
}

fn finish_guardrails_ai(
    mock: Option<MockProvider>,
    result: Result<Option<String>, GuardrailsAiGuardrailError>,
) -> HelperResult {
    finish(mock, result, |body, updated_text| {
        body.insert("updated_text".to_owned(), json!(updated_text));
    })
}

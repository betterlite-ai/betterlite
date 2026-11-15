use super::GUARDRAIL_NAME;
use crate::generic_guardrail_contracts::GenericGuardrailAPIResponse;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

pub(super) fn inputs_from_response(response: GenericGuardrailAPIResponse) -> GuardrailApiInputs {
    // the generic API already puts the caller's rows back where the scan returned none
    let images = response.images.filter(|values| !values.is_empty());
    let tools = response.tools.filter(|values| !values.is_empty());
    GuardrailApiInputs {
        texts: Some(response.texts.unwrap_or_default()),
        images,
        tools,
        tool_calls: None,
        structured_messages: response.structured_messages,
        model: None,
        stream_holdback_chars: response
            .stream_holdback_chars
            .map(|values| values.into_iter().map(i64::from).collect()),
    }
}

pub(super) fn blocked_failure(reason: &str) -> GuardrailFailure {
    let message = match reason.strip_prefix(": ") {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => "Content violates policy".to_owned(),
    };
    GuardrailFailure::GuardrailRaised {
        guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
        message,
        wrap_default_message: false,
        status_code: 400,
        blocked_content: true,
    }
}

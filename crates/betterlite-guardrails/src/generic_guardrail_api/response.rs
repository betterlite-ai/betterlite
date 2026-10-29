use crate::generic_guardrail_api::GenericGuardrailAPIError;
use crate::generic_guardrail_contracts::{
    GenericGuardrailAPIRequest, GenericGuardrailAPIResponse, GuardrailMessage,
    GuardrailResponseAction, reconcile_structured_rows,
};

pub(super) fn passthrough_response(
    request: &GenericGuardrailAPIRequest,
) -> GenericGuardrailAPIResponse {
    let texts = request.texts.clone();
    let images = request.images.clone();
    let tools = request.tools.clone();
    let structured_messages = request.structured_messages.clone();
    GenericGuardrailAPIResponse {
        action: GuardrailResponseAction::None,
        texts,
        images,
        tools,
        structured_messages,
        blocked_reason: None,
        stream_holdback_chars: None,
    }
}

fn is_empty<T>(values: Option<&Vec<T>>) -> bool {
    values.is_none_or(Vec::is_empty)
}

fn field_or_original<T>(value: Option<Vec<T>>, original: Option<&Vec<T>>) -> Option<Vec<T>>
where
    T: Clone,
{
    if is_empty(value.as_ref()) {
        original.cloned()
    } else {
        value
    }
}

fn reconcile_structured_messages(
    response_rows: Option<Vec<GuardrailMessage>>,
    original: &GenericGuardrailAPIRequest,
    shown: &GenericGuardrailAPIRequest,
) -> Option<Vec<GuardrailMessage>> {
    if is_empty(response_rows.as_ref()) {
        return None;
    }
    if let (Some(original_rows), Some(shown_rows)) = (
        original.structured_messages.as_deref(),
        shown.structured_messages.as_deref(),
    ) {
        reconcile_structured_rows(
            original_rows,
            shown_rows,
            response_rows.as_deref().unwrap_or_default(),
        )
    } else {
        response_rows
    }
}

pub(super) fn enforce_response(
    response: GenericGuardrailAPIResponse,
    original: &GenericGuardrailAPIRequest,
    shown: &GenericGuardrailAPIRequest,
) -> Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError> {
    if response.action == GuardrailResponseAction::Blocked {
        let reason = response
            .blocked_reason
            .as_deref()
            .map_or_else(String::new, |value| format!(": {value}"));
        return Err(GenericGuardrailAPIError::Blocked { reason });
    }
    let texts = field_or_original(response.texts, original.texts.as_ref());
    let images = field_or_original(response.images, original.images.as_ref());
    let tools = field_or_original(response.tools, original.tools.as_ref());
    let structured_messages =
        reconcile_structured_messages(response.structured_messages, original, shown);
    Ok(GenericGuardrailAPIResponse {
        action: response.action,
        texts,
        images,
        tools,
        structured_messages,
        blocked_reason: response.blocked_reason,
        stream_holdback_chars: response.stream_holdback_chars,
    })
}

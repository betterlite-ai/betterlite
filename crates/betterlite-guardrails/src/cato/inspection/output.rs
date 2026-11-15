use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{CallerArgs, call_id, inspection_messages, synthetic_message};
use crate::cato::actions::{block_error, policy_names};
use crate::cato::error::CatoError;
use crate::cato::headers::build_cato_headers;
use crate::cato::transport::post_analyze;
use crate::cato::wire::{AnalyzeRequest, AnalyzeResponse};
use crate::cato::{CallHook, CatoGuardrail};

pub(crate) mod fragments;

use fragments::{response_fragments, write_fragment};

// the response body shape LiteLLM's post-call hook was handed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseKind {
    Chat,
    Responses,
}

// LiteLLM's async_post_call_success_hook: every assistant fragment is inspected in order and redacted in place
pub async fn post_call_success_hook(
    guardrail: &CatoGuardrail,
    request_data: &BTreeMap<String, GuardrailValue>,
    caller: CallerArgs<'_>,
    kind: ResponseKind,
    response: &mut GuardrailValue,
) -> Result<(), CatoError> {
    for fragment in response_fragments(response, kind) {
        let inspected =
            call_cato_guardrail_on_output(guardrail, request_data, caller, &fragment.text).await?;
        if let Some(redacted) = inspected {
            write_fragment(response, &fragment.path, redacted)?;
        }
    }
    Ok(())
}

// LiteLLM's call_cato_guardrail_on_output: the assistant turn is appended after the inspected request
async fn call_cato_guardrail_on_output(
    guardrail: &CatoGuardrail,
    request_data: &BTreeMap<String, GuardrailValue>,
    caller: CallerArgs<'_>,
    output: &str,
) -> Result<Option<String>, CatoError> {
    let mut messages = inspection_messages(request_data);
    let assistant_index = messages.len();
    messages.push(synthetic_message("assistant", output));
    let headers = build_cato_headers(
        guardrail,
        CallHook::Output,
        caller.key_alias,
        caller.user_email,
        call_id(request_data),
    )?;
    let response = post_analyze(guardrail, &AnalyzeRequest { messages }, headers).await?;
    let Some(required_action) = response.required_action.as_ref() else {
        return Ok(None);
    };
    if required_action.action_type.as_deref() == Some("block_action") {
        tracing::info!(
            detected = true,
            policies = ?policy_names(&response),
            "Cato: detected"
        );
        return Err(block_error(required_action));
    }
    if required_action.action_type.as_deref() != Some("anonymize_action") {
        return Ok(None);
    }
    redacted_assistant_text(&response, assistant_index)
}

// LiteLLM reads the assistant turn back out of all_redacted_messages by the index it was sent at
fn redacted_assistant_text(
    response: &AnalyzeResponse,
    assistant_index: usize,
) -> Result<Option<String>, CatoError> {
    let content = response
        .redacted_chat
        .as_ref()
        .and_then(|chat| chat.all_redacted_messages.as_ref())
        .and_then(|redacted| redacted.get(assistant_index))
        .and_then(|message| message.content.as_ref());
    match content {
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(CatoError::OutputRedactionNotText),
        None => Ok(None),
    }
}

use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::{GuardrailInputType, UnreachableFallback};
use crate::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::straiker::delivery::attempts::RetryDelay;
use crate::straiker::delivery::transport::WebhookPost;
use crate::straiker::delivery::{ApplyRequest, StraikerDelivery, phase_name};

const BLOCK_STATUS: u16 = 400;

// LiteLLM's hidden.setdefault("straiker", {}).update({action, turn_id}) on the response object.
pub fn record_verdict(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    action: &str,
    turn_id: Option<&str>,
) {
    let Some(GuardrailValue::Object(response)) = request_data.get_mut("response") else {
        return;
    };
    let Some(GuardrailValue::Object(hidden)) = response.get_mut("_hidden_params") else {
        return;
    };
    let GuardrailValue::Object(straiker) = hidden
        .entry("straiker".to_owned())
        .or_insert_with(|| GuardrailValue::Object(BTreeMap::new()))
    else {
        return;
    };
    straiker.insert("action".to_owned(), GuardrailValue::Text(action.to_owned()));
    let turn = turn_id.map_or(GuardrailValue::Null, |text| {
        GuardrailValue::Text(text.to_owned())
    });
    straiker.insert("turn_id".to_owned(), turn);
}

impl<Post, Delay> StraikerDelivery<Post, Delay>
where
    Post: WebhookPost,
    Delay: RetryDelay,
{
    pub(super) fn block(
        &self,
        request: &ApplyRequest<'_>,
        message: &str,
        blocked_content: bool,
    ) -> GuardrailFailure {
        match request.input_type {
            GuardrailInputType::Request => GuardrailFailure::GuardrailRaised {
                guardrail_name: Some(self.settings.guardrail_name.clone()),
                message: message.to_owned(),
                wrap_default_message: false,
                status_code: BLOCK_STATUS,
                blocked_content,
            },
            GuardrailInputType::Response => {
                let model = match request.request_data.get("model") {
                    Some(GuardrailValue::Text(name)) => name.clone(),
                    Some(_) | None => "unknown".to_owned(),
                };
                GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
                    message: message.to_owned(),
                    model,
                    request_data: request.request_data.clone(),
                    guardrail_name: Some(self.settings.guardrail_name.clone()),
                    detection_info: BTreeMap::new(),
                    original_response: request.request_data.get("response").cloned(),
                }))
            }
        }
    }

    // LiteLLM's fallback: let the call through when the endpoint is unreachable under fail_open or when fail_on_error is off, block otherwise.
    pub(super) fn fail(
        &self,
        request: &ApplyRequest<'_>,
        error: &dyn Display,
        is_unreachable: bool,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let fail_open = (is_unreachable
            && self.settings.unreachable_fallback == UnreachableFallback::FailOpen)
            || !self.settings.fail_on_error;
        tracing::error!(
            event = "straiker.error",
            input_type = phase_name(request.input_type),
            error = %error,
            fail_open,
            "straiker.error"
        );
        if fail_open {
            return Ok(request.inputs.clone());
        }
        let message = format!("Straiker detection unavailable: {error}");
        Err(self.block(request, &message, false))
    }
}

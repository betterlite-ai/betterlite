use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    GlobalSkipFlags, GuardrailSkipFlags,
};

use crate::sse_error_frames::exception_detail_parts;

pub mod anthropic;
pub mod chat;
pub mod responses;
pub mod stamps;
pub mod tables;
pub mod undeliverable;

mod tests;

pub(crate) const UNKNOWN_GUARDRAIL_NAME: &str = "unknown";

// LiteLLM's guardrail object also carries the caller, the configured name and the skip flags its methods read
pub struct GuardrailToApply<G> {
    pub guardrail: Arc<G>,
    pub guardrail_name: Option<String>,
    pub principal: Principal,
    pub skip_flags: GuardrailSkipFlags,
    pub global_flags: GlobalSkipFlags,
}

impl<G: GuardrailContract + Send + Sync> GuardrailContract for GuardrailToApply<G> {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.guardrail
            .apply_guardrail(inputs, request_data, input_type, logging_obj)
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        self.guardrail.get_config_model()
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        self.guardrail.get_supported_event_hooks()
    }

    fn supports_tool_result_scan(&self) -> bool {
        self.guardrail.supports_tool_result_scan()
    }

    fn messages_cover_full_request(&self) -> bool {
        self.guardrail.messages_cover_full_request()
    }
}

// The status and detail LiteLLM's sse_error_payload and build_stream_error_items read off the exception
#[must_use]
pub(crate) fn failure_parts(failure: &GuardrailFailure) -> Option<(u16, JsonValue)> {
    match failure {
        GuardrailFailure::Http { status, detail } => {
            Some((*status, JsonValue::String(detail.clone())))
        }
        GuardrailFailure::HttpObject { status, detail } => {
            let fields: BTreeMap<String, GuardrailValue> = detail
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let encoded = serde_json::to_value(GuardrailValue::Object(fields)).ok()?;
            Some((*status, encoded))
        }
        GuardrailFailure::ModifyResponse(details) => {
            Some((400, JsonValue::String(details.message.clone())))
        }
        GuardrailFailure::GuardrailRaised { status_code, .. }
        | GuardrailFailure::BlockedPii { status_code, .. } => {
            Some((*status_code, JsonValue::String(failure.to_string())))
        }
        GuardrailFailure::SensitiveDataRoute(_) | GuardrailFailure::Other(_) => None,
    }
}

// LiteLLM's serialize_http_exception_detail: the message an in-stream error frame carries
#[must_use]
pub(crate) fn failure_message(failure: &GuardrailFailure) -> Option<(u16, String)> {
    let (status, detail) = failure_parts(failure)?;
    let (message, _) = exception_detail_parts(&detail);
    Some((status, message))
}

// Only a ModifyResponse block synthesizes a terminator stream; every other failure travels as an error frame
#[must_use]
pub(crate) fn modify_response(failure: &GuardrailFailure) -> Option<&ModifyResponseFailure> {
    match failure {
        GuardrailFailure::ModifyResponse(details) => Some(details),
        GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Http { .. }
        | GuardrailFailure::HttpObject { .. }
        | GuardrailFailure::Other(_) => None,
    }
}

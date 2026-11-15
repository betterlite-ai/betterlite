use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::invocation::GuardrailInvocation;
use crate::guardrail_records::record::GuardrailSpanSink;
use crate::model_armor::config::ModelArmorGuardrail;
use crate::model_armor::decisions::model_response::CallResponse;
use crate::model_armor::transport::ModelArmorTransport;
use crate::model_armor::transport::tokens::ArmorTokens;

pub mod applied_guardrails;
pub mod error_handling;
pub mod events;
pub mod logging;
pub mod logging_information;
pub mod logging_only;
pub mod masking;
pub mod metadata;
pub mod moderation;
pub mod post_call;
pub mod pre_call;
pub mod request_masking;
pub mod response_masking;
mod timing;

use error_handling::HookError;
use events::HookPhase;

pub const MODEL_ARMOR_PROVIDER: &str = "model_armor";

pub type Data = BTreeMap<String, GuardrailValue>;

// What one hook call needs from the dispatcher besides the request data
pub struct HookContext<'context> {
    pub suppressed: &'context BTreeSet<String>,
    pub invocation: &'context mut GuardrailInvocation,
    pub sink: &'context (dyn GuardrailSpanSink + Sync),
    pub log_target: Option<&'context mut GuardrailLogTarget>,
    pub model_call_details: Option<&'context mut BTreeMap<String, GuardrailValue>>,
}

// The fields of LiteLLM's ModelArmorGuardrail instance the hooks read, plus the transport they scan through
pub struct ModelArmorHooks<'guardrail, T: ArmorTokens> {
    pub guardrail: &'guardrail ModelArmorGuardrail,
    pub transport: &'guardrail ModelArmorTransport<T>,
    pub fail_on_error: bool,
    pub skip_unscannable_attachments: bool,
    pub clock: fn() -> SystemTime,
}

impl<T: ArmorTokens> ModelArmorHooks<'_, T> {
    pub async fn pre_call(
        &self,
        data: &mut Data,
        call_type: Option<&str>,
        context: &mut HookContext<'_>,
    ) -> Result<(), HookError<T::Error>> {
        let scanned = pre_call::scan(self, data, HookPhase::PreCall, call_type, context).await;
        error_handling::hook_outcome(scanned, self.fail_on_error, "pre-call")?;
        logging::process_response(self, data, context)
    }

    pub async fn moderation(
        &self,
        data: &mut Data,
        call_type: Option<&str>,
        context: &mut HookContext<'_>,
    ) -> Result<(), HookError<T::Error>> {
        let scanned = pre_call::scan(self, data, moderation::PHASE, call_type, context).await;
        error_handling::hook_outcome(scanned, self.fail_on_error, "moderation")?;
        logging::process_response(self, data, context)
    }

    pub async fn post_call_success(
        &self,
        data: &mut Data,
        response: &mut CallResponse,
        context: &mut HookContext<'_>,
    ) -> Result<(), HookError<T::Error>> {
        let scanned = post_call::scan(self, data, response, context).await;
        error_handling::hook_outcome(scanned, self.fail_on_error, "post-call")?;
        logging::process_response(self, data, context)
    }
}

#[cfg(test)]
mod tests;

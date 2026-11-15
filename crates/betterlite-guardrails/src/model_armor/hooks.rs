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

// LiteLLM's pre-call and moderation hooks run the same scan, and only the event phase differs
macro_rules! request_hook {
    ($name:ident, $phase:expr) => {
        pub async fn $name(
            &self,
            data: &mut Data,
            call_type: Option<&str>,
            context: &mut HookContext<'_>,
        ) -> Result<(), HookError<T::Error>> {
            let scanned = pre_call::scan(self, data, $phase, call_type, context).await;
            self.finish(data, scanned, $phase, context)
        }
    };
}

impl<T: ArmorTokens> ModelArmorHooks<'_, T> {
    request_hook!(pre_call, HookPhase::PreCall);
    request_hook!(moderation, moderation::PHASE);

    pub async fn post_call_success(
        &self,
        data: &mut Data,
        response: &mut CallResponse,
        context: &mut HookContext<'_>,
    ) -> Result<(), HookError<T::Error>> {
        let scanned = post_call::scan(self, data, response, context).await;
        error_handling::hook_outcome(scanned, self.fail_on_error, "post-call")?;
        self.process_response(data, context)
    }

    // LiteLLM's `except` clauses around a hook and its _process_response call on the way out
    fn finish(
        &self,
        data: &mut Data,
        scanned: Result<(), HookError<T::Error>>,
        phase: HookPhase,
        context: &mut HookContext<'_>,
    ) -> Result<(), HookError<T::Error>> {
        error_handling::hook_outcome(scanned, self.fail_on_error, phase.hook_name())?;
        self.process_response(data, context)
    }
}

mod tests;

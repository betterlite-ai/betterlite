use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_records::invocation::GuardrailInvocation;
use crate::guardrail_records::record::GuardrailSpanSink;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::model_armor::config::ModelArmorGuardrail;
use crate::model_armor::decisions::model_response::CallResponse;
use crate::model_armor::decisions::{
    build_block_error_detail, build_logging_response, should_block_content,
};
use crate::model_armor::transport::ArmorResponse;
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
use logging_information::ApplyVerdict;

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
        logging::process_response(self, data, context)
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
        logging::process_response(self, data, context)
    }

    // LiteLLM's apply_guardrail: scan, record the verdict, and block only outside logging_only mode
    pub async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        data: &mut Data,
        input_type: GuardrailInputType,
        context: &mut HookContext<'_>,
    ) -> Result<GuardrailApiInputs, HookError<T::Error>> {
        let text = logging_only::joined_texts(&inputs);
        if text.is_empty() {
            logging::process_response(self, data, context)?;
            return Ok(inputs);
        }
        self.scan_and_record(data, input_type, context, &text)
            .await?;
        Ok(inputs)
    }

    // The scan apply_guardrail sends: the request, its timing, and the verdict it records
    async fn scan_and_record(
        &self,
        data: &mut Data,
        input_type: GuardrailInputType,
        context: &mut HookContext<'_>,
        text: &str,
    ) -> Result<(), HookError<T::Error>> {
        let started_at = (self.clock)();
        let result = self
            .transport
            .make_model_armor_request(Some(text), logging_only::source(input_type), None)
            .await;
        let times = timing::between(started_at, (self.clock)());
        let armor = match result {
            Ok(armor) => armor,
            Err(error) => {
                let verdict = logging_only::failure_verdict(error)?;
                return logging_information::record_apply_guardrail(
                    self, data, context, verdict, times,
                );
            }
        };
        let flagged = should_block_content(&armor.verdict, false);
        let response = build_logging_response(&armor.body, self.guardrail.sanitize_error_detail);
        let verdict = if flagged {
            ApplyVerdict::Flagged { response }
        } else {
            ApplyVerdict::Success { response }
        };
        logging_information::record_apply_guardrail(self, data, context, verdict, times)?;
        self.block_when_flagged(input_type, &armor, flagged)
    }

    // The block half of apply_guardrail: a flagged verdict becomes a 400 unless logging_only mode is on
    fn block_when_flagged(
        &self,
        input_type: GuardrailInputType,
        armor: &ArmorResponse,
        flagged: bool,
    ) -> Result<(), HookError<T::Error>> {
        if !flagged
            || self
                .guardrail
                .base
                .event_hook_is_event_type(GuardrailEventHooks::LoggingOnly)
        {
            return Ok(());
        }
        Err(HookError::Blocked {
            status: 400,
            detail: build_block_error_detail(
                logging_only::block_message(input_type),
                &armor.body,
                self.guardrail.sanitize_error_detail,
            ),
        })
    }
}

mod tests;

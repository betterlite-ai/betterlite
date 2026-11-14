use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::enkryptai_detection::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use error::EnkryptAiError;
use logging::{RecordRequest, Timing};
use transport::{Parsed, Response};
use wire::DetectionResponse;

pub mod error;
mod logging;
mod message;
pub mod process;
mod transport;
pub mod wire;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_records::record::{GuardrailSpanSink, GuardrailStatus};
use process::Processed;

pub const GUARDRAIL_PROVIDER: &str = "enkryptai";
pub const API_PATH: &str = "/guardrails/policy/detect";

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 3] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
];

// EnkryptAI defines apply_guardrail and sets none of CustomGuardrail's native-hook class variables
const ENKRYPTAI_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: false,
    uses_apply_guardrail_interface: true,
    deny_text_on_error: false,
};

pub struct EnkryptAiGuardrail {
    pub base: GuardrailBase,
    pub settings: Configured,
    pub client: reqwest::Client,
    pub clock: fn() -> SystemTime,
    pub sink: Arc<dyn GuardrailSpanSink + Send + Sync>,
}

pub(crate) struct Detection {
    pub(crate) response: DetectionResponse,
    pub(crate) processed: Processed,
}

impl EnkryptAiGuardrail {
    pub fn new(
        configured: Configured,
        client: reqwest::Client,
        clock: fn() -> SystemTime,
        sink: Arc<dyn GuardrailSpanSink + Send + Sync>,
        strict_modes: bool,
    ) -> Result<Self, EnkryptAiError> {
        let settings = GuardrailSettings {
            guardrail_name: Some(configured.guardrail_name.clone()),
            supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
            event_hook: configured.mode.clone(),
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: Some(configured.timeout.as_secs_f64()),
            switches: ENKRYPTAI_SWITCHES,
        };
        let base = GuardrailBase::new(settings, strict_modes)
            .map_err(|source| EnkryptAiError::Base { source })?;
        Ok(Self {
            base,
            settings: configured,
            client,
            clock,
            sink,
        })
    }

    pub async fn apply(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        mut logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        for text in inputs.texts.iter().flatten() {
            let started = (self.clock)();
            let response = transport::send(self, text).await;
            let timing = Timing {
                started,
                finished: (self.clock)(),
            };
            let detection = self
                .finish_response(response, request_data, logging_obj.as_deref_mut(), timing)
                .map_err(|error| self.failure(error))?;
            if detection.processed.attacks.is_empty() {
                continue;
            }
            let message = message::detected(&detection.processed, &detection.response)
                .map_err(|source| self.failure(EnkryptAiError::Serialize { source }))?;
            return Err(self.failure(EnkryptAiError::Detected { message }));
        }
        Ok(inputs)
    }

    fn finish_response(
        &self,
        response: Response,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        log_target: Option<&mut GuardrailLogTarget>,
        timing: Timing,
    ) -> Result<Detection, EnkryptAiError> {
        let Response { raw, parsed: result } = response;
        let (outcome, status) = match result {
            Parsed::Response(body) => {
                let processed = process::process(&body);
                let status = processed.status();
                let detection = Detection {
                    response: *body,
                    processed,
                };
                (Ok(detection), status)
            }
            Parsed::Malformed(error) => (Err(error), GuardrailStatus::GuardrailFailedToRespond),
        };
        logging::record(RecordRequest {
            guardrail: self,
            request_data,
            log_target,
            response: raw,
            status,
            timing,
        })?;
        outcome
    }

    fn failure(&self, error: EnkryptAiError) -> GuardrailFailure {
        match error {
            EnkryptAiError::Detected { message } => GuardrailFailure::GuardrailRaised {
                guardrail_name: self.base.settings.guardrail_name.clone(),
                message,
                wrap_default_message: false,
                status_code: 400,
                blocked_content: true,
            },
            other @ (EnkryptAiError::HttpStatus { .. }
            | EnkryptAiError::Request { .. }
            | EnkryptAiError::Timeout { .. }
            | EnkryptAiError::ResponseTooLarge { .. }
            | EnkryptAiError::ResponseJson { .. }
            | EnkryptAiError::MalformedResponse
            | EnkryptAiError::ResponseShape { .. }
            | EnkryptAiError::InvalidHeader { .. }
            | EnkryptAiError::Record { .. }
            | EnkryptAiError::Base { .. }
            | EnkryptAiError::Serialize { .. }) => GuardrailFailure::Other(other.to_string()),
        }
    }
}

impl GuardrailContract for EnkryptAiGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        _input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.apply(inputs, request_data, logging_obj)
    }
}

#[path = "enkryptai/tests.rs"]
mod tests;

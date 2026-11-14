use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_records::record::GuardrailStatus;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::noma_scan_policy::error::NomaBlockedMessage;

use super::SUPPORTED_EVENT_HOOKS;
use super::action::{self, Action};
use super::config::NomaV2Settings;
use super::error::{NomaV2Error, blocked_failure};
use super::payload::{self, ScanRequest};
use super::records::{self, ScanRun};
use super::transport;

#[derive(Debug, Clone)]
pub struct NomaV2Guardrail {
    pub base: GuardrailBase,
    pub settings: NomaV2Settings,
    pub client: reqwest::Client,
    pub clock: fn() -> SystemTime,
}

struct ScanOutcome {
    status: GuardrailStatus,
    response: GuardrailValue,
    effect: Effect,
}

// What the scan and its record leave for apply_guardrail to hand back
struct ScanFinish {
    status: GuardrailStatus,
    response: GuardrailValue,
    result: Result<GuardrailApiInputs, GuardrailFailure>,
}

enum Effect {
    Unchanged,
    Modified(Box<GuardrailApiInputs>),
    Blocked(NomaBlockedMessage),
}

impl NomaV2Guardrail {
    async fn scan(&self, request: &ScanRequest<'_>) -> Result<ScanOutcome, NomaV2Error> {
        let body = {
            let payload = payload::build(request, &self.settings);
            payload::wire_payload(&payload)
        };
        let response = transport::call(self, &body).await?;
        if self.settings.monitor_mode {
            tracing::debug!("Noma v2 guardrail decision: action=NONE (monitor mode)");
            return Ok(ScanOutcome {
                status: GuardrailStatus::Success,
                response,
                effect: Effect::Unchanged,
            });
        }
        let action = action::parse(&response)?;
        tracing::debug!("Noma v2 guardrail decision: action={action:?}");
        let effect = match action {
            Action::Blocked => Effect::Blocked(NomaBlockedMessage {
                details: response.clone(),
            }),
            Action::None => Effect::Unchanged,
            Action::GuardrailIntervened => Effect::Modified(Box::new(action::intervene(
                request.inputs.clone(),
                &response,
            )?)),
        };
        let status = if action == Action::None {
            GuardrailStatus::Success
        } else {
            GuardrailStatus::GuardrailIntervened
        };
        Ok(ScanOutcome {
            status,
            response,
            effect,
        })
    }

    // LiteLLM resolves the dynamic params before its try block, so their error carries no record
    fn resolved_application_id(
        &self,
        request_data: &BTreeMap<String, GuardrailValue>,
    ) -> Result<Option<String>, GuardrailFailure> {
        let dynamic = self
            .base
            .dynamic_request_body_params(request_data)
            .map_err(GuardrailFailure::from)?;
        Ok(payload::application_id(
            &dynamic,
            self.settings.application_id.as_deref(),
            request_data,
        ))
    }
}

fn finish_outcome(
    outcome: Result<ScanOutcome, NomaV2Error>,
    inputs: GuardrailApiInputs,
    block_failures: bool,
) -> ScanFinish {
    match outcome {
        Ok(scanned) => ScanFinish {
            status: scanned.status,
            response: scanned.response,
            result: match scanned.effect {
                Effect::Unchanged => Ok(inputs),
                Effect::Modified(updated) => Ok(*updated),
                Effect::Blocked(message) => Err(blocked_failure(&message)),
            },
        },
        Err(error) => {
            tracing::error!("Noma v2 guardrail failed: {error}");
            let response = GuardrailValue::Text(error.to_string());
            let result = if block_failures {
                Err(error.into_failure())
            } else {
                Ok(inputs)
            };
            ScanFinish {
                status: GuardrailStatus::GuardrailFailedToRespond,
                response,
                result,
            }
        }
    }
}

impl GuardrailContract for NomaV2Guardrail {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let started = (self.clock)();
        let application_id = self.resolved_application_id(request_data)?;
        let outcome = {
            let request = ScanRequest {
                inputs: &inputs,
                request_data,
                input_type,
                log_target: logging_obj.as_deref(),
                application_id,
            };
            self.scan(&request).await
        };
        let finished = finish_outcome(outcome, inputs, self.settings.block_failures);
        let ended = (self.clock)();
        let run = ScanRun {
            response: finished.response,
            status: finished.status,
            started,
            ended,
        };
        match records::record_scan(self, request_data, logging_obj, run) {
            Ok(()) => finished.result,
            Err(error) => Err(error.into_failure()),
        }
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}

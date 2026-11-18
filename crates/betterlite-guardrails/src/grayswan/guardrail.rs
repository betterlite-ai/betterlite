use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::grayswan::{Configured, OnFlaggedAction};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::Client;

use crate::generic_guardrail_contracts::{GuardrailExtraValue, GuardrailInputType};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_selection::GuardrailSelection;
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::third_party_init_a::grayswan::GraySwanArgs;

use super::construction::{
    DEFAULT_FAIL_OPEN, GraySwanGuardrailSetupError, resolve_api_base, resolve_api_key,
    resolve_on_flagged_action, resolve_reasoning_mode, resolve_sampling_rate, resolve_threshold,
    resolve_timeout,
};
use super::error::{GRAYSWAN_BLOCK_ERROR_MSG, GraySwanFailure, GraySwanGuardrailAPIError};
use super::format::{build_messages, format_violation_message};
use super::grayswan_monitoring::RequestContext;
use super::grayswan_monitoring::call::call_grayswan_api;
use super::grayswan_monitoring::failure::{GuardrailFailureLog, log_guardrail_failure};
use super::grayswan_monitoring::helpers::{GuardrailException, is_grayswan_exception};
use super::grayswan_monitoring::payload::{PreparePayloadRequest, prepare_payload};
use super::hooks::{SUPPORTED_EVENT_HOOKS, hook_name};
use super::wire::{
    GraySwanInput, GraySwanPayload, GraySwanResponse, GraySwanResult, GraySwanVerdict,
    GraySwanViolatedRule,
};

type Object = BTreeMap<String, GuardrailValue>;

impl GraySwanGuardrail {
    pub fn new(
        args: GraySwanArgs,
        base: GuardrailBase,
        client: Client,
        env: &dyn EnvironmentLookup,
    ) -> Result<Self, GraySwanGuardrailSetupError> {
        let settings = Configured {
            guardrail_name: args.name,
            api_key: resolve_api_key(args.api_key.as_ref(), env)?,
            api_base: resolve_api_base(args.api_base.as_deref(), env),
            on_flagged_action: resolve_on_flagged_action(args.on_flagged_action.as_deref()),
            violation_threshold: resolve_threshold(args.violation_threshold, env),
            reasoning_mode: resolve_reasoning_mode(args.reasoning_mode.as_deref(), env),
            categories: args.categories.unwrap_or_default(),
            policy_id: args.policy_id.unwrap_or_default(),
            fail_open: args.fail_open.unwrap_or(DEFAULT_FAIL_OPEN),
            guardrail_timeout: resolve_timeout(args.guardrail_timeout)?,
            streaming_end_of_stream_only: args.streaming_end_of_stream_only,
            streaming_sampling_rate: resolve_sampling_rate(args.streaming_sampling_rate)?,
        };
        let base = initialized_base(base, &settings.guardrail_name);
        tracing::debug!(
            "GraySwan __init__: streaming_end_of_stream_only={}, streaming_sampling_rate={}",
            settings.streaming_end_of_stream_only,
            settings.streaming_sampling_rate
        );
        Ok(Self {
            settings,
            base,
            client,
        })
    }

    pub fn should_run_guardrail(
        &self,
        data: &Object,
        event_type: GuardrailEventHooks,
        suppressed: &BTreeSet<String>,
    ) -> Result<bool, GuardrailSelectionError> {
        let result = self
            .base
            .should_run_guardrail(data, event_type, suppressed)?;
        tracing::debug!(
            event_type = hook_name(event_type),
            result,
            event_hook = ?self.base.settings.event_hook,
            has_apply_guardrail = true,
            class = "GraySwanGuardrail",
            "GraySwan DEBUG: should_run_guardrail"
        );
        Ok(result)
    }

    #[must_use]
    pub fn get_supported_event_hooks() -> Vec<GuardrailEventHooks> {
        SUPPORTED_EVENT_HOOKS.to_vec()
    }

    pub async fn apply_guardrail(
        &self,
        mut input: GraySwanInput,
        input_type: GuardrailInputType,
        request: RequestContext<'_>,
    ) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
        let Some(payload) = self.prepare_call(&mut input, input_type, &request) else {
            return Ok(GraySwanResult {
                verdict: GraySwanVerdict::Pass,
                texts: input.texts,
                tool_calls: input.tool_calls,
            });
        };
        let started_at = SystemTime::now();
        let result = match call_grayswan_api(&self.client, &self.settings, &payload).await {
            Ok(response) => {
                self.process_response(&response, input_type, &input.texts, &input.tool_calls)
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(result) => Ok(result),
            Err(error) => {
                self.recover_failure(error, request, started_at)?;
                Ok(GraySwanResult {
                    verdict: GraySwanVerdict::Pass,
                    texts: input.texts,
                    tool_calls: input.tool_calls,
                })
            }
        }
    }

    // LiteLLM scans tool calls and sends prior messages and tools only on the response path
    fn prepare_call(
        &self,
        input: &mut GraySwanInput,
        input_type: GuardrailInputType,
        request: &RequestContext<'_>,
    ) -> Option<GraySwanPayload> {
        let is_response = input_type == GuardrailInputType::Response;
        let scanned_tool_calls: &[GuardrailExtraValue] =
            if is_response { &input.tool_calls } else { &[] };
        if input.texts.is_empty() && scanned_tool_calls.is_empty() {
            return None;
        }
        let context = if is_response {
            std::mem::take(&mut input.request_history)
        } else {
            Vec::new()
        };
        let tools = if is_response {
            std::mem::take(&mut input.tools)
        } else {
            Vec::new()
        };
        let messages = build_messages(&input.texts, scanned_tool_calls, input_type, context);
        let payload = prepare_payload(
            &self.settings,
            PreparePayloadRequest {
                messages,
                tools: (!tools.is_empty()).then_some(tools),
                dynamic: &input.metadata,
                request_data: &*request.request_data,
                log_target: request.log_target.as_deref(),
            },
        );
        Some(payload)
    }

    fn recover_failure(
        &self,
        error: GraySwanGuardrailAPIError,
        request: RequestContext<'_>,
        started_at: SystemTime,
    ) -> Result<(), GraySwanGuardrailAPIError> {
        if is_grayswan_exception(GuardrailException::Api(&error)) {
            return Err(error);
        }
        log_guardrail_failure(
            &self.base,
            GuardrailFailureLog {
                message: error.to_string(),
                status_code: error.status_code,
                started_at,
                finished_at: SystemTime::now(),
                request_data: request.request_data,
                log_target: request.log_target,
            },
        );
        if self.settings.fail_open {
            tracing::warn!(
                error = %error,
                "Gray Swan Guardrail: fail_open=True. Allowing request to proceed despite error"
            );
            return Ok(());
        }
        Err(error)
    }

    pub fn process_response(
        &self,
        response: &GraySwanResponse,
        input_type: GuardrailInputType,
        texts: &[String],
        tool_calls: &[GuardrailExtraValue],
    ) -> Result<GraySwanResult, GraySwanGuardrailAPIError> {
        let flagged = response.violation >= self.settings.violation_threshold;
        if !flagged {
            return Ok(GraySwanResult {
                verdict: GraySwanVerdict::Pass,
                texts: texts.to_vec(),
                tool_calls: tool_calls.to_vec(),
            });
        }
        let is_output = input_type == GuardrailInputType::Response;
        let violation_message = format_violation_message(response, is_output);
        match self.settings.on_flagged_action {
            OnFlaggedAction::Monitor => {
                tracing::info!("Gray Swan Guardrail: Monitoring mode - allowing flagged content");
                Ok(GraySwanResult {
                    verdict: GraySwanVerdict::Monitored { violation_message },
                    texts: texts.to_vec(),
                    tool_calls: tool_calls.to_vec(),
                })
            }
            OnFlaggedAction::Block => Err(blocked_error(response, &violation_message, is_output)),
            OnFlaggedAction::Passthrough if is_output => Ok(GraySwanResult {
                verdict: GraySwanVerdict::Replaced {
                    violation_message: violation_message.clone(),
                },
                texts: vec![violation_message],
                tool_calls: tool_calls.to_vec(),
            }),
            OnFlaggedAction::Passthrough => Ok(GraySwanResult {
                verdict: GraySwanVerdict::ShortCircuited { violation_message },
                texts: texts.to_vec(),
                tool_calls: tool_calls.to_vec(),
            }),
        }
    }
}

fn initialized_base(mut base: GuardrailBase, name: &str) -> GuardrailBase {
    if base.settings.guardrail_name.is_none() {
        base.settings.guardrail_name = Some(name.to_owned());
    }
    base.settings.supported_event_hooks = Some(SUPPORTED_EVENT_HOOKS.to_vec());
    base
}

fn blocked_error(
    response: &GraySwanResponse,
    violation_message: &str,
    is_output: bool,
) -> GraySwanGuardrailAPIError {
    let violated_rules = response
        .violated_rule_descriptions
        .iter()
        .map(GraySwanViolatedRule::to_extra_value)
        .collect();
    GraySwanGuardrailAPIError::from_failure(GraySwanFailure::Blocked {
        reason: format!("{GRAYSWAN_BLOCK_ERROR_MSG}: {violation_message}"),
        violation_location: if is_output { "output" } else { "input" },
        violation: response.violation,
        violated_rules,
        mutation: response.mutation,
        ipi: response.ipi,
    })
}

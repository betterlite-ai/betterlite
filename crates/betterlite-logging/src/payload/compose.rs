use betterlite_config::sections::logging_privacy::Settings;
use betterlite_error::normalize::Normalizer;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};

use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::{LoggingStatus, get_status_fields};
use crate::logging_metadata::base64::Base64Truncator;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_metadata::setup::merge_litellm_metadata;
use crate::logging_state::LoggingState;
use crate::payload::deps::{PayloadSettings, PayloadSources};
use crate::payload::error::{PayloadError, guardrail_information};
use crate::spend_bodies::redaction::should_redact_message_logging;

mod costs;
mod error_info;
mod identifiers;
mod inputs;
mod proxy_request;
mod response_messages;
mod standard_metadata;
mod usage_timing;

// What one build reads off the call: the state and the handler's own arguments.
pub struct PayloadRequest<'input> {
    pub state: &'input LoggingState,
    pub init_response_obj: &'input Json,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub status: LoggingStatus,
    pub error_str: Option<&'input str>,
    pub original_exception: Option<&'input ExceptionFacts>,
    pub standard_built_in_tools_params: Option<&'input Json>,
}

// The injected sources and policies one builder holds for every payload it writes.
pub struct PayloadComposition<'input> {
    pub sources: &'input PayloadSources,
    pub settings: &'input PayloadSettings,
    pub redactor: &'input Redactor,
    pub normalizer: &'input Normalizer,
    pub truncator: &'input Base64Truncator,
    pub privacy: Settings,
}

// get_standard_logging_object_payload over the typed state; an Err is the Python catch-all's None.
#[rustfmt::skip]
pub fn standard_logging_payload(
    request: &PayloadRequest<'_>, composition: &PayloadComposition<'_>,
) -> Result<StandardLoggingPayload, PayloadError> {
    let params = inputs::litellm_params(request.state)?;
    let metadata = merge_litellm_metadata(&params);
    let extracted = inputs::extracted_response(request)?;
    let usage = usage_timing::usage_object(request, &metadata, composition)?;
    let totals = usage_timing::totals(&usage)?;
    let timing = usage_timing::timing(request)?;
    let identifiers = identifiers::Identifiers::read(request, &params, &metadata, composition)?;
    let proxy = proxy_request::ProxyRequest::read(&request.state.litellm_params)?;
    let clean_metadata = standard_metadata::standard_metadata(&standard_metadata::StandardMetadataInput { request, params: &params, metadata: &metadata, usage_object: &usage, identifiers: &identifiers }, composition)?;
    let guardrails = guardrail_information(metadata.get("standard_logging_guardrail_information"))?;
    let redact = should_redact_message_logging(
        Some(&request.state.litellm_params),
        Some(&request.state.callback_dynamic_params),
        composition.privacy.turn_off_message_logging,
    );
    let response = response_messages::final_response(request, &extracted, redact);
    let model = response_messages::model_name(request, &extracted, &identifiers.model, &response);
    let costs = costs::Costs::read(&costs::CostsInput { request, metadata: &metadata, usage: &usage, identifiers: &identifiers, model: &model }, composition)?;
    let hidden_params = costs::hidden_params(request, &extracted, costs.raw_response_cost)?;
    let (error_information, error_str) = error_info::build(request, &metadata, composition)?;
    let messages = response_messages::logged_messages(request, composition, redact)?;
    let model_parameters = response_messages::model_parameters(request.state)?;
    let request_tags = proxy_request::request_tags(&params, &proxy, composition)?;
    let end_user = clean_metadata.user_api_key_end_user_id.clone().or(proxy.body_user);
    let requester_ip_address = clean_metadata.requester_ip_address.clone();
    let user_agent = clean_metadata.user_agent.clone();
    Ok(StandardLoggingPayload {
        classifier_input: None, originating_request_masked: None,
        id: identifiers.id, trace_id: identifiers.trace_id, session_id: identifiers.session_id,
        litellm_call_id: Some(identifiers.call_id), call_type: request.state.call_type.clone(),
        stream: identifiers.stream, response_cost: costs.response_cost,
        cost_breakdown: costs.cost_breakdown, autorouter_savings: costs.autorouter_savings,
        autorouter_savings_estimate: costs.autorouter_savings_estimate,
        baseline_observation: costs.baseline_observation,
        cost_failure_info: request.state.model_call_details.get("response_cost_failure_debug_information").cloned(),
        zero_cost_diagnostic: Some(request.state.model_call_details.get("zero_cost_diagnostic").cloned().unwrap_or(Json::Null)),
        status: request.status, status_fields: get_status_fields(request.status, guardrails.as_deref()),
        custom_llm_provider: request.state.custom_llm_provider.clone(),
        total_tokens: totals.total, prompt_tokens: totals.prompt, completion_tokens: totals.completion,
        start_time: timing.start_time, end_time: timing.end_time,
        completion_start_time: timing.completion_start_time, response_time: timing.response_time,
        model_map_information: costs.model_map_information, model,
        model_id: Some(identifiers.model_id), model_group: Some(identifiers.model_group),
        api_base: identifiers.api_base, metadata: clean_metadata,
        cache_hit: identifiers.cache_hit, cache_key: hidden_params.cache_key.clone(),
        saved_cache_cost: costs.saved_cache_cost, request_tags,
        request_model_access_groups: Some(composition.sources.access_groups.access_groups(&request.state.litellm_params).unwrap_or_default()),
        end_user, requester_ip_address, user_agent, messages, response: Some(response),
        error_str, error_information: Some(error_information), model_parameters, hidden_params,
        guardrail_information: guardrails,
        standard_built_in_tools_params: request.standard_built_in_tools_params.cloned(),
    })
}

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_error::normalize::Normalizer;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};

use self::render::Render;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::{GuardrailInformation, StandardLoggingMetadata};
use crate::fields::response::ExtractedResponse;
use crate::fields::status::{LoggingStatus, get_status_fields};
use crate::logging_metadata::base64::Base64Truncator;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_metadata::params::{LitellmParams, MetadataMap};
use crate::logging_metadata::setup::merge_litellm_metadata;
use crate::logging_state::LoggingState;
use crate::payload::deps::{PayloadSettings, PayloadSources};
use crate::payload::error::{PayloadError, guardrail_information};

mod costs;
pub(crate) mod error_info;
mod identifiers;
mod inputs;
mod proxy_request;
mod render;
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

// The call as read: everything the render step needs about the request and its response.
struct Basis {
    params: LitellmParams,
    metadata: MetadataMap,
    extracted: ExtractedResponse,
    usage: Json,
    totals: usage_timing::Totals,
    timing: usage_timing::Timing,
    identifiers: identifiers::Identifiers,
    proxy: proxy_request::ProxyRequest,
    clean_metadata: StandardLoggingMetadata,
    guardrails: Option<Vec<GuardrailInformation>>,
}

impl Basis {
    fn read(
        request: &PayloadRequest<'_>,
        composition: &PayloadComposition<'_>,
    ) -> Result<Self, PayloadError> {
        let params = inputs::litellm_params(request.state)?;
        let metadata = merge_litellm_metadata(&params);
        let extracted = inputs::extracted_response(request);
        let usage = usage_timing::usage_object(request, &metadata, composition)?;
        let totals = usage_timing::totals(&usage)?;
        let timing = usage_timing::timing(request)?;
        let identifiers = identifiers::Identifiers::read(request, &params, &metadata, composition)?;
        let proxy = proxy_request::ProxyRequest::read(&request.state.litellm_params)?;
        let clean_metadata = standard_metadata::standard_metadata(
            &standard_metadata::StandardMetadataInput {
                request,
                params: &params,
                metadata: &metadata,
                usage_object: &usage,
                identifiers: &identifiers,
            },
            composition,
        )?;
        let guardrails =
            guardrail_information(metadata.get("standard_logging_guardrail_information"))?;
        Ok(Self {
            params,
            metadata,
            extracted,
            usage,
            totals,
            timing,
            identifiers,
            proxy,
            clean_metadata,
            guardrails,
        })
    }
}

// get_standard_logging_object_payload over the typed state; an Err is the Python catch-all's None.
pub fn standard_logging_payload(
    request: &PayloadRequest<'_>,
    composition: &PayloadComposition<'_>,
) -> Result<StandardLoggingPayload, PayloadError> {
    let basis = Basis::read(request, composition)?;
    let render = Render::read(request, composition, &basis)?;
    Ok(assemble(request, composition, basis, render))
}

#[rustfmt::skip]
fn assemble(
    request: &PayloadRequest<'_>, composition: &PayloadComposition<'_>,
    basis: Basis, render: Render,
) -> StandardLoggingPayload {
    StandardLoggingPayload {
        classifier_input: None, originating_request_masked: None,
        id: basis.identifiers.id, trace_id: basis.identifiers.trace_id, session_id: basis.identifiers.session_id,
        litellm_call_id: Some(basis.identifiers.call_id), call_type: request.state.call_type.clone(),
        stream: basis.identifiers.stream, response_cost: render.costs.response_cost,
        cost_breakdown: render.costs.cost_breakdown, autorouter_savings: render.costs.autorouter_savings,
        autorouter_savings_estimate: render.costs.autorouter_savings_estimate,
        baseline_observation: render.costs.baseline_observation,
        cost_failure_info: request.state.model_call_details.get("response_cost_failure_debug_information").cloned(),
        zero_cost_diagnostic: Some(request.state.model_call_details.get("zero_cost_diagnostic").cloned().unwrap_or(Json::Null)),
        status: request.status, status_fields: get_status_fields(request.status, basis.guardrails.as_deref()),
        custom_llm_provider: request.state.custom_llm_provider.clone(),
        total_tokens: basis.totals.total, prompt_tokens: basis.totals.prompt, completion_tokens: basis.totals.completion,
        start_time: basis.timing.start, end_time: basis.timing.end,
        completion_start_time: basis.timing.completion_start, response_time: basis.timing.response,
        model_map_information: render.costs.model_map_information, model: render.model,
        model_id: Some(basis.identifiers.model_id), model_group: Some(basis.identifiers.model_group),
        api_base: basis.identifiers.api_base, metadata: basis.clean_metadata,
        cache_hit: basis.identifiers.cache_hit, cache_key: render.hidden_params.cache_key.clone(),
        saved_cache_cost: render.costs.saved_cache_cost, request_tags: render.request_tags,
        request_model_access_groups: Some(composition.sources.access_groups.access_groups(&request.state.litellm_params).unwrap_or_default()),
        end_user: render.end_user, requester_ip_address: render.requester_ip_address, user_agent: render.user_agent,
        messages: render.messages, response: Some(render.response),
        error_str: render.error_str, error_information: Some(render.error_information),
        model_parameters: render.model_parameters, hidden_params: render.hidden_params,
        guardrail_information: basis.guardrails,
        standard_built_in_tools_params: request.standard_built_in_tools_params.cloned(),
    }
}

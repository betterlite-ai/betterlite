use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;
use http::header::HeaderMap;
use uuid::Uuid;

use crate::gemini_candidates::blocked::complete_block;
use crate::gemini_candidates::candidate::Candidate;
use crate::gemini_candidates::choice::Choices;
use crate::gemini_candidates::process::{Mode, process_candidates};
use crate::gemini_metadata::extension::MetadataExtension;
use crate::gemini_response::decode::{GenerateContentResponse, decode_generate_content};
use crate::gemini_response::error::{GenerateContentError, header_text};
use crate::gemini_response_parts::projection::{CallMode, PartContext};
use crate::gemini_usage::usage::{apply_grounding_counters, calculate_usage};
use crate::model_policy::error::VertexAiError;

pub const SERVICE_TIER_HEADER: &str = "x-gemini-service-tier";
const STANDARD_SERVICE_TIER: &str = "standard";
const DEFAULT_SERVICE_TIER: &str = "default";

#[derive(Debug)]
pub struct RawResponse<'raw> {
    pub headers: &'raw HeaderMap,
    pub body: &'raw [u8],
}

#[derive(Debug)]
pub struct CompleteRequest<'request> {
    pub model: &'request str,
    pub now: i64,
    pub call_mode: CallMode,
}

// the complete path sets these four lists and never the streamed vertex_ai_safety_ratings copy
#[derive(Debug, Default)]
pub struct CompleteMetadata {
    pub grounding_metadata: Vec<Json>,
    pub url_context_metadata: Vec<Json>,
    pub safety_results: Vec<Json>,
    pub citation_metadata: Vec<Json>,
}

#[derive(Debug)]
pub struct ProviderFields {
    pub traffic_type: String,
}

#[derive(Debug)]
pub struct CompleteResponse {
    pub id: String,
    pub model: String,
    pub choices: Choices,
    pub usage: Usage,
    pub metadata: Option<CompleteMetadata>,
    pub provider_specific_fields: Option<ProviderFields>,
    pub service_tier: Option<String>,
}

pub fn transform_response(
    raw: &RawResponse<'_>,
    request: &CompleteRequest<'_>,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<CompleteResponse, VertexAiError> {
    let body =
        decode_generate_content(raw.body).map_err(|error| error.into_vertex_error(raw.headers))?;
    assemble_complete_response(&body, raw.headers, request, ids)
        .map_err(|error| error.into_vertex_error(raw.headers))
}

pub fn assemble_complete_response(
    body: &GenerateContentResponse,
    headers: &HeaderMap,
    request: &CompleteRequest<'_>,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<CompleteResponse, GenerateContentError> {
    let model = served_model_name(body.model_version.as_ref())
        .unwrap_or(request.model)
        .to_owned();
    let id = format!("{CHAT_COMPLETION_ID_PREFIX}{}", ids());
    if let Some(blocked) = complete_block(
        body.prompt_feedback.as_ref(),
        body.candidates.as_deref().unwrap_or_default(),
        body.usage_metadata.as_ref(),
    )? {
        // the blocked reply keeps the served model, the generated id and the raw counters only
        return Ok(CompleteResponse {
            id,
            model,
            choices: Choices::Complete(vec![blocked.choice]),
            usage: blocked.usage,
            metadata: None,
            provider_specific_fields: None,
            service_tier: None,
        });
    }
    let (choices, usage, metadata) = project_ordinary(body, request, ids)?;
    Ok(CompleteResponse {
        id,
        model,
        choices,
        usage,
        metadata: Some(metadata),
        provider_specific_fields: body
            .traffic_type
            .clone()
            .map(|traffic_type| ProviderFields { traffic_type }),
        service_tier: service_tier(headers),
    })
}

fn project_ordinary(
    body: &GenerateContentResponse,
    request: &CompleteRequest<'_>,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<(Choices, Usage, CompleteMetadata), GenerateContentError> {
    let (choices, extension, usage_candidates) = project_candidates(
        body.candidates.as_deref().unwrap_or_default(),
        PartContext {
            cumulative_tool_call_index: 0,
            call_mode: request.call_mode,
            now: request.now,
        },
        ids,
    )?;
    let mut usage = calculate_usage(body.usage_metadata.as_ref(), &usage_candidates)?;
    apply_grounding_counters(&mut usage, &extension.grounding_metadata);
    Ok((
        choices,
        usage,
        CompleteMetadata {
            grounding_metadata: extension.grounding_metadata,
            url_context_metadata: extension.url_context_metadata,
            safety_results: extension.safety_results,
            citation_metadata: extension.citation_metadata,
        },
    ))
}

fn project_candidates(
    candidates: &[Candidate],
    context: PartContext,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<(Choices, MetadataExtension, Vec<Json>), GenerateContentError> {
    if candidates.is_empty() {
        return Ok((
            Choices::Complete(Vec::new()),
            MetadataExtension::default(),
            Vec::new(),
        ));
    }
    let processed = process_candidates(candidates, Mode::Complete, context, ids)?;
    let usage_candidates = candidates
        .iter()
        .map(|candidate| Json::Object(candidate.metadata.clone()))
        .collect();
    Ok((processed.choices, processed.metadata, usage_candidates))
}

// LiteLLM splits the runtime modelVersion before its first @, and keeps the requested model otherwise
#[must_use]
pub fn served_model_name(model_version: Option<&Json>) -> Option<&str> {
    match model_version? {
        Json::Text(version) if !version.is_empty() => Some(
            version
                .split_once('@')
                .map_or(version.as_str(), |(name, _)| name),
        ),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}

#[must_use]
fn service_tier(headers: &HeaderMap) -> Option<String> {
    let lowered = header_text(headers, SERVICE_TIER_HEADER)?.to_lowercase();
    if lowered.is_empty() {
        return None;
    }
    if lowered == STANDARD_SERVICE_TIER {
        return Some(DEFAULT_SERVICE_TIER.to_owned());
    }
    Some(lowered)
}

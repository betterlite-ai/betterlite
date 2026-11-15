use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

use crate::feature_flags::FeatureParams;
use crate::feature_flags::compaction::requires_compaction_beta;
use crate::headers::AnthropicHeaders;
use crate::tools::wire::ADVISOR_TYPE;

pub const ANTHROPIC_BETA_HEADER: &str = "anthropic-beta";
pub const WEB_FETCH_BETA: &str = "web-fetch-2025-09-10";
pub const CONTEXT_MANAGEMENT_BETA: &str = "context-management-2025-06-27";
pub const COMPACT_BETA: &str = "compact-2026-01-12";
pub const NATIVE_COMPACT_BETA: &str = "compact-2026-09-04";
pub const STRUCTURED_OUTPUT_BETA: &str = "structured-outputs-2025-11-13";
pub const FAST_MODE_BETA: &str = "fast-mode-2026-02-01";
pub const ADVISOR_TOOL_BETA: &str = "advisor-tool-2026-03-01";

const COMPACT_EDIT: &str = "compact_20260112";
const COMPACTION_EDIT: &str = "compaction";
const CONTEXT_MANAGEMENT: &str = "context_management";
const EDITS: &str = "edits";
const FAST_SPEED: &str = "fast";
const FORMAT: &str = "format";
const MEMORY: &str = "memory";
const OUTPUT_CONFIG: &str = "output_config";
const OUTPUT_FORMAT: &str = "output_format";
const SPEED: &str = "speed";
const TYPE: &str = "type";
const VERTEX_REQUEST: &str = "is_vertex_request";
const WEB_FETCH: &str = "web_fetch";

#[derive(Debug, thiserror::Error)]
pub enum BetaHeaderError {
    #[error("context_management.edits must be a list")]
    EditsNotList,
    #[error("context_management edit {index} is not an object")]
    EditNotObject { index: usize },
}

impl BetaHeaderError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::EditsNotList | Self::EditNotObject { .. } => Some(400),
        }
    }
}

// Every casing of the header merges into one lowercase value, first occurrence winning its position
pub fn ensure_beta_header(headers: &mut AnthropicHeaders, beta_value: &str) {
    let mut merged: Vec<String> = Vec::new();
    for (name, value) in &headers.values {
        if name.to_lowercase() != ANTHROPIC_BETA_HEADER {
            continue;
        }
        for beta in value.split(',') {
            let trimmed = beta.trim();
            if trimmed.is_empty() {
                continue;
            }
            push_unique(&mut merged, trimmed);
        }
    }
    push_unique(&mut merged, beta_value);
    let variants: Vec<String> = headers
        .values
        .keys()
        .filter(|name| name.to_lowercase() == ANTHROPIC_BETA_HEADER)
        .cloned()
        .collect();
    for variant in variants {
        headers.values.shift_remove(&variant);
    }
    headers
        .values
        .insert(ANTHROPIC_BETA_HEADER.to_owned(), merged.join(", "));
}

pub fn add_context_management_beta(
    headers: &mut AnthropicHeaders,
    context_management: &Json,
) -> Result<(), BetaHeaderError> {
    let edits: &[Json] = match context_management {
        Json::List(items) => items,
        Json::Object(fields) => match fields.get(EDITS) {
            None | Some(Json::Null) => return Ok(()),
            Some(Json::List(items)) => items,
            Some(_) => return Err(BetaHeaderError::EditsNotList),
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => return Ok(()),
    };
    let (compact, managed) = classify_edits(edits)?;
    if compact {
        ensure_beta_header(headers, COMPACT_BETA);
    }
    if managed {
        ensure_beta_header(headers, CONTEXT_MANAGEMENT_BETA);
    }
    Ok(())
}

pub fn add_optional_betas(
    headers: &mut AnthropicHeaders,
    params: &IndexMap<String, Json>,
    tools: &[PromptJson],
    messages: &[PromptJson],
    provider: &str,
) -> Result<(), BetaHeaderError> {
    if matches!(params.get(VERTEX_REQUEST), Some(Json::Bool(true))) {
        return Ok(());
    }
    add_compaction_beta(headers, params, messages, provider);
    add_tool_betas(headers, tools);
    match params.get(CONTEXT_MANAGEMENT) {
        None | Some(Json::Null) => {}
        Some(value) => add_context_management_beta(headers, value)?,
    }
    add_structured_output_beta(headers, params);
    add_speed_beta(headers, params);
    add_advisor_beta(headers, tools);
    Ok(())
}

fn push_unique(values: &mut Vec<String>, candidate: &str) {
    if values.iter().any(|value| value == candidate) {
        return;
    }
    values.push(candidate.to_owned());
}

fn classify_edits(edits: &[Json]) -> Result<(bool, bool), BetaHeaderError> {
    let mut compact = false;
    let mut managed = false;
    for (index, edit) in edits.iter().enumerate() {
        let Some(fields) = object(edit) else {
            return Err(BetaHeaderError::EditNotObject { index });
        };
        match fields.get(TYPE).and_then(text) {
            Some(COMPACT_EDIT | COMPACTION_EDIT) => compact = true,
            Some(_) | None => managed = true,
        }
    }
    Ok((compact, managed))
}

fn add_compaction_beta(
    headers: &mut AnthropicHeaders,
    params: &IndexMap<String, Json>,
    messages: &[PromptJson],
    provider: &str,
) {
    let compaction = match params.get("compaction") {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    };
    let feature_params = FeatureParams {
        compaction,
        container: None,
        output_config: None,
        reasoning_effort: None,
    };
    if requires_compaction_beta(provider, &feature_params, messages) {
        ensure_beta_header(headers, NATIVE_COMPACT_BETA);
    }
}

fn add_tool_betas(headers: &mut AnthropicHeaders, tools: &[PromptJson]) {
    for tool in tools {
        match tool_kind(tool) {
            Some(kind) if kind.starts_with(WEB_FETCH) => {
                ensure_beta_header(headers, WEB_FETCH_BETA)
            }
            Some(kind) if kind.starts_with(MEMORY) => {
                ensure_beta_header(headers, CONTEXT_MANAGEMENT_BETA);
            }
            Some(_) | None => {}
        }
    }
}

fn add_structured_output_beta(headers: &mut AnthropicHeaders, params: &IndexMap<String, Json>) {
    let legacy = params
        .get(OUTPUT_FORMAT)
        .is_some_and(|value| !matches!(value, Json::Null));
    let nested = match params.get(OUTPUT_CONFIG) {
        Some(Json::Object(fields)) => fields
            .get(FORMAT)
            .is_some_and(|value| !matches!(value, Json::Null)),
        Some(_) | None => false,
    };
    if legacy || nested {
        ensure_beta_header(headers, STRUCTURED_OUTPUT_BETA);
    }
}

fn add_speed_beta(headers: &mut AnthropicHeaders, params: &IndexMap<String, Json>) {
    if matches!(params.get(SPEED), Some(Json::Text(value)) if value == FAST_SPEED) {
        ensure_beta_header(headers, FAST_MODE_BETA);
    }
}

fn add_advisor_beta(headers: &mut AnthropicHeaders, tools: &[PromptJson]) {
    if tools
        .iter()
        .any(|tool| tool_kind(tool) == Some(ADVISOR_TYPE))
    {
        ensure_beta_header(headers, ADVISOR_TOOL_BETA);
    }
}

pub(super) fn tool_kind(tool: &PromptJson) -> Option<&str> {
    let fields = tool.as_object()?;
    fields.get(TYPE).and_then(PromptJson::as_str)
}

fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(value) => Some(value.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

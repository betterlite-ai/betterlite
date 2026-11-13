use std::collections::BTreeSet;

use betterlite_anthropic::feature_flags::FeatureParams;
use betterlite_anthropic::feature_flags::beta::{
    THINKING_DISPLAY_BETA, TOOL_CHANGES_BETA, tool_search_beta_header,
};
use betterlite_anthropic::feature_flags::compaction::requires_compaction_beta;
use betterlite_anthropic::feature_flags::messages::{
    has_output_config, has_thinking_display_updates, has_tool_change,
};
use betterlite_anthropic::feature_flags::tools::is_tool_search_used;
use betterlite_models::decisions::Json as ParamsJson;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;

use super::Headers;
use super::error::BetaFilterError;

pub const ADVISOR_TOOL_BETA: &str = "advisor-tool-2026-03-01";
pub const COMPACT_BETA: &str = "compact-2026-01-12";
pub const COMPACT_NATIVE_BETA: &str = "compact-2026-09-04";
pub const CONTEXT_MANAGEMENT_BETA: &str = "context-management-2025-06-27";
pub const FAST_MODE_BETA: &str = "fast-mode-2026-02-01";
pub const PER_TURN_CONTROL_BETA: &str = "per-turn-control-2026-07-01";
pub const STRUCTURED_OUTPUTS_BETA: &str = "structured-outputs-2025-11-13";
const ADVISOR_TOOL_TYPE: &str = "advisor_20260301";
const COMPACT_EDIT_TYPE: &str = "compact_20260112";
const BETA_HEADER: &str = "anthropic-beta";

pub struct NativeRequest<'request> {
    pub provider: &'request str,
    pub optional_params: &'request IndexMap<String, Json>,
    pub messages: &'request [Json],
}

pub fn inject_native_betas(
    headers: Headers,
    request: &NativeRequest<'_>,
) -> Result<Headers, BetaFilterError> {
    let betas = request_betas(&headers, request)?;
    if betas.is_empty() {
        return Ok(headers);
    }
    let mut merged: Headers = headers
        .into_iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case(BETA_HEADER))
        .collect();
    let tokens: Vec<&str> = betas.iter().map(String::as_str).collect();
    merged.insert(BETA_HEADER.to_owned(), tokens.join(","));
    Ok(merged)
}

fn request_betas(
    headers: &Headers,
    request: &NativeRequest<'_>,
) -> Result<BTreeSet<String>, BetaFilterError> {
    let mut betas = existing_beta_tokens(headers);
    let compaction = optional_object(request.optional_params, "compaction")?;
    let feature_params = FeatureParams {
        compaction: compaction.map(params_json),
        container: None,
        output_config: None,
        reasoning_effort: None,
    };
    if requires_compaction_beta(request.provider, &feature_params, request.messages) {
        betas.insert(COMPACT_NATIVE_BETA.to_owned());
    }
    add_context_management_betas(&mut betas, request.optional_params)?;
    if structured_outputs_used(request.optional_params)? {
        betas.insert(STRUCTURED_OUTPUTS_BETA.to_owned());
    }
    if fast_mode_used(request.optional_params)? {
        betas.insert(FAST_MODE_BETA.to_owned());
    }
    if has_output_config(request.messages) {
        betas.insert(PER_TURN_CONTROL_BETA.to_owned());
    }
    let tools = tools(request.optional_params)?;
    if tools.iter().any(is_advisor_tool) {
        betas.insert(ADVISOR_TOOL_BETA.to_owned());
    }
    if is_tool_search_used(tools) {
        betas.insert(tool_search_beta_header(request.provider).to_owned());
    }
    if has_thinking_display_updates(optional_object(request.optional_params, "thinking")?) {
        betas.insert(THINKING_DISPLAY_BETA.to_owned());
    }
    if has_tool_change(request.messages) {
        betas.insert(TOOL_CHANGES_BETA.to_owned());
    }
    Ok(betas)
}

fn existing_beta_tokens(headers: &Headers) -> BTreeSet<String> {
    let mut betas = BTreeSet::new();
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case(BETA_HEADER) {
            continue;
        }
        for piece in value.split(',') {
            let token = piece.trim();
            if !token.is_empty() {
                betas.insert(token.to_owned());
            }
        }
    }
    betas
}

fn context_management_edits(
    params: &IndexMap<String, Json>,
) -> Result<Option<&[Json]>, BetaFilterError> {
    let Some(management) = optional_object(params, "context_management")? else {
        return Ok(None);
    };
    match management
        .as_object()
        .and_then(|fields| fields.get("edits"))
    {
        None => Ok(Some(&[])),
        Some(Json::List(edits)) => Ok(Some(edits)),
        Some(_) => Err(BetaFilterError::Field {
            field: "context_management.edits",
            expected: "a list",
        }),
    }
}

fn add_context_management_betas(
    betas: &mut BTreeSet<String>,
    params: &IndexMap<String, Json>,
) -> Result<(), BetaFilterError> {
    let Some(edits) = context_management_edits(params)? else {
        return Ok(());
    };
    let mut compact = false;
    let mut other = false;
    for edit in edits {
        let Some(fields) = edit.as_object() else {
            return Err(BetaFilterError::Field {
                field: "context_management.edits",
                expected: "a list of objects",
            });
        };
        if fields.get("type").and_then(Json::as_str) == Some(COMPACT_EDIT_TYPE) {
            compact = true;
        } else {
            other = true;
        }
    }
    if compact {
        betas.insert(COMPACT_BETA.to_owned());
    }
    if other {
        betas.insert(CONTEXT_MANAGEMENT_BETA.to_owned());
    }
    Ok(())
}

fn structured_outputs_used(params: &IndexMap<String, Json>) -> Result<bool, BetaFilterError> {
    if optional_object(params, "output_format")?.is_some() {
        return Ok(true);
    }
    Ok(optional_object(params, "output_config")?
        .and_then(Json::as_object)
        .and_then(|config| config.get("format"))
        .is_some_and(|format| !matches!(format, Json::Null)))
}

fn fast_mode_used(params: &IndexMap<String, Json>) -> Result<bool, BetaFilterError> {
    match params.get("speed") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Text(speed)) => Ok(speed == "fast"),
        Some(_) => Err(BetaFilterError::Field {
            field: "speed",
            expected: "a string",
        }),
    }
}

fn tools(params: &IndexMap<String, Json>) -> Result<&[Json], BetaFilterError> {
    match params.get("tools") {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::List(tools)) => Ok(tools),
        Some(_) => Err(BetaFilterError::Field {
            field: "tools",
            expected: "a list",
        }),
    }
}

fn is_advisor_tool(tool: &Json) -> bool {
    tool.as_object()
        .and_then(|fields| fields.get("type"))
        .and_then(Json::as_str)
        == Some(ADVISOR_TOOL_TYPE)
}

fn optional_object<'value>(
    params: &'value IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<&'value Json>, BetaFilterError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value @ Json::Object(_)) => Ok(Some(value)),
        Some(_) => Err(BetaFilterError::Field {
            field,
            expected: "an object",
        }),
    }
}

// the compaction predicate reads its own params type, so the decoded value crosses over unchanged
fn params_json(value: &Json) -> ParamsJson {
    match value {
        Json::Null => ParamsJson::Null,
        Json::Bool(flag) => ParamsJson::Bool(*flag),
        Json::Number(number) => ParamsJson::Number(number.clone()),
        Json::Text(text) => ParamsJson::Text(text.clone()),
        Json::List(items) => ParamsJson::List(items.iter().map(params_json).collect()),
        Json::Object(fields) => ParamsJson::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), params_json(item)))
                .collect(),
        ),
    }
}

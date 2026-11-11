use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::messages_context_clear::blocks::{clear_tool_results, tool_ids};
use crate::api::messages_context_clear::constants::{
    CLEAR_TOOL_USES_EDIT_TYPE, DEFAULT_INPUT_TOKENS_TRIGGER, DEFAULT_KEEP_TOOL_USES,
};
use crate::api::messages_context_clear::counter::{TokenCountRequest, TokenCounter};
use crate::api::messages_context_clear::error::ClearToolUsesError;
use crate::api::messages_context_clear::result::AppliedEdit;
use crate::api::messages_context_clear::spec::{ClearCountSpec, ClearToolUsesEditSpec};

pub struct EditorRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<Json>,
    pub tools: Option<&'request [Json]>,
    // the dispatcher passes system to every editor, but this one never counts or edits it
    pub system: Option<&'request Json>,
    pub edit_spec: &'request ClearToolUsesEditSpec,
}

pub struct ClearToolUsesOutcome {
    pub messages: Vec<Json>,
    pub applied: Option<AppliedEdit>,
    pub warnings: Vec<String>,
}

fn is_tool_uses(kind: Option<&Json>) -> bool {
    let Some(Json::Text(text)) = kind else {
        return false;
    };
    text == "tool_uses"
}

fn keep_is_tool_uses(kind: Option<&Option<Json>>) -> bool {
    match kind {
        None => true,
        Some(Some(Json::Text(text))) => text == "tool_uses",
        Some(_) => false,
    }
}

// an integer beyond i128 keeps its side of the range: a huge threshold can never be exceeded and a huge keep keeps everything
fn integer_value(value: &Json) -> Option<i128> {
    match value {
        Json::Bool(true) => Some(1),
        Json::Bool(false) => Some(0),
        Json::Number(number) => number_value(number),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn number_value(number: &Number) -> Option<i128> {
    if let Some(value) = number.as_i128() {
        return Some(value);
    }
    let literal = number.to_string();
    let (negative, magnitude) = literal
        .strip_prefix('-')
        .map_or((false, literal.as_str()), |magnitude| (true, magnitude));
    if !magnitude.chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    Some(if negative { i128::MIN } else { i128::MAX })
}

fn exceeds(count: usize, threshold: i128) -> bool {
    if threshold < 0 {
        return true;
    }
    match usize::try_from(threshold) {
        Ok(limit) => count > limit,
        Err(_too_large) => false,
    }
}

fn resolved_keep_count(keep: &ClearCountSpec) -> usize {
    let default = usize::try_from(DEFAULT_KEEP_TOOL_USES).unwrap_or(usize::MAX);
    if !keep_is_tool_uses(keep.kind.as_ref()) {
        return default;
    }
    match keep.value.as_ref().and_then(integer_value) {
        Some(value) if value >= 0 => usize::try_from(value).unwrap_or(usize::MAX),
        _ => default,
    }
}

fn ignored_knob_warnings(edit_spec: &ClearToolUsesEditSpec) -> (Vec<&'static str>, Vec<String>) {
    let mut ignored = Vec::new();
    let mut warnings = Vec::new();
    let present = [
        ("clear_at_least", edit_spec.clear_at_least.is_some()),
        ("exclude_tools", edit_spec.exclude_tools.is_some()),
        ("clear_tool_inputs", edit_spec.clear_tool_inputs.is_some()),
    ];
    for (knob, is_present) in present {
        if !is_present {
            continue;
        }
        let message = format!(
            "context_management polyfill: ignoring '{knob}' on {CLEAR_TOOL_USES_EDIT_TYPE} (supported only on Anthropic-family forwarding path in v0)"
        );
        tracing::warn!("{message}");
        ignored.push(knob);
        warnings.push(message);
    }
    (ignored, warnings)
}

fn trigger_met(
    trigger: &ClearCountSpec,
    request: &EditorRequest<'_>,
    counter: &dyn TokenCounter,
) -> Result<(bool, Option<usize>), ClearToolUsesError> {
    if is_tool_uses(trigger.kind.as_ref().and_then(|kind| kind.as_ref())) {
        let Some(threshold) = trigger.value.as_ref().and_then(integer_value) else {
            return Ok((false, None));
        };
        let count = tool_ids(&request.messages).uses.len();
        return Ok((exceeds(count, threshold), None));
    }
    let default_threshold = i128::from(DEFAULT_INPUT_TOKENS_TRIGGER);
    let threshold = trigger
        .value
        .as_ref()
        .and_then(integer_value)
        .unwrap_or(default_threshold);
    let current = counter.count(TokenCountRequest {
        model: request.model,
        messages: &request.messages,
        tools: request.tools,
    })?;
    tracing::debug!("context_management polyfill: current_tokens: {current}");
    tracing::debug!("context_management polyfill: threshold: {threshold}");
    Ok((exceeds(current, threshold), Some(current)))
}

fn selected_ids<'message>(
    ids: &[&'message str],
    last_result: Option<&'message str>,
    keep_count: usize,
) -> BTreeSet<&'message str> {
    let mut selected: BTreeSet<&str> = ids
        .iter()
        .take(ids.len().saturating_sub(keep_count))
        .copied()
        .collect();
    if let Some(last) = last_result {
        selected.remove(last);
    }
    selected
}

fn applied_edit(
    cleared: usize,
    cleared_input_tokens: usize,
    ignored: &[&'static str],
) -> AppliedEdit {
    AppliedEdit {
        edit_type: Some(CLEAR_TOOL_USES_EDIT_TYPE.to_owned()),
        cleared_input_tokens: Some(cleared_input_tokens),
        cleared_tool_uses: Some(cleared),
        cleared_thinking_turns: None,
        summary_input_tokens: None,
        summary_output_tokens: None,
        error: None,
        warnings: (!ignored.is_empty()).then(|| {
            ignored
                .iter()
                .map(|knob| format!("{knob}_ignored"))
                .collect()
        }),
    }
}

pub fn apply_clear_tool_uses_20250919(
    request: EditorRequest<'_>,
    counter: &dyn TokenCounter,
) -> Result<ClearToolUsesOutcome, ClearToolUsesError> {
    let (ignored, warnings) = ignored_knob_warnings(request.edit_spec);
    let (messages, applied) = clear_messages(request, counter, &ignored)?;
    Ok(ClearToolUsesOutcome {
        messages,
        applied,
        warnings,
    })
}

fn count_tokens(
    counter: &dyn TokenCounter,
    request: &EditorRequest<'_>,
    messages: &[Json],
) -> Result<usize, ClearToolUsesError> {
    counter.count(TokenCountRequest {
        model: request.model,
        messages,
        tools: request.tools,
    })
}

fn clear_messages(
    request: EditorRequest<'_>,
    counter: &dyn TokenCounter,
    ignored: &[&'static str],
) -> Result<(Vec<Json>, Option<AppliedEdit>), ClearToolUsesError> {
    let trigger = request.edit_spec.trigger.clone().unwrap_or_default();
    let keep = request.edit_spec.keep.clone().unwrap_or_default();
    let (met, tokens_before) = trigger_met(&trigger, &request, counter)?;
    if !met {
        return Ok((request.messages, None));
    }
    let keep_count = resolved_keep_count(&keep);
    let collected = tool_ids(&request.messages);
    if collected.uses.len() <= keep_count {
        return Ok((request.messages, None));
    }
    let selected = selected_ids(&collected.uses, collected.last_result, keep_count);
    let (edited, cleared) = clear_tool_results(&request.messages, &selected);
    tracing::debug!("context_management polyfill: edited: {edited:?}");
    if cleared == 0 {
        return Ok((request.messages, None));
    }
    let before = match tokens_before {
        Some(count) => count,
        None => count_tokens(counter, &request, &request.messages)?,
    };
    let after = count_tokens(counter, &request, &edited)?;
    Ok((
        edited,
        Some(applied_edit(cleared, before.saturating_sub(after), ignored)),
    ))
}

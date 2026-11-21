use betterlite_models::call_helpers::tool_calls::remove_index_from_tool_calls;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::output_limits::{ErrorBody, RejectedFieldsError, without_rejected_fields};
use indexmap::IndexMap;
use regex::Regex;
use thiserror::Error;

// LiteLLM's AzureFoundryErrorStrings.SET_EXTRA_PARAMETERS_TO_PASS_THROUGH
pub const PASS_THROUGH_HINT: &str = "Set extra-parameters to 'pass-through'";
// the first marker should_retry_llm_api_inside_llm_translation_on_http_error matches
pub const EXTRA_INPUTS_MARKER: &str = "Extra inputs are not permitted";
// the second marker the same policy always retries on
pub const INVALID_TOOL_CALL_INDEX_MARKER: &str =
    "unknown field: parameter index is not a valid field";
// max_retry_on_unprocessable_entity_error: LiteLLM's range(max(2, 1)) makes two sends, so one repair
pub const MAX_ATTEMPTS: u32 = 2;

// LiteLLM's re.search(r"tools\[\d+\]\."): \d is the decimal-digit category
const TOOL_INDEX_PATTERN: &str = r"tools\[\p{Decimal_Number}+\]\.";
// LiteLLM's re.findall(r"tools\[\d+\]\.([\w-]+)"): \w is Python's str.isalnum letter and number categories plus the underscore
const TOOL_FIELD_PATTERN: &str = r"tools\[\p{Decimal_Number}+\]\.([\p{L}\p{N}_-]+)";
// LiteLLM's re.search(r"\[(.*?)\]"): the dot stops at a newline
const BRACKET_PATTERN: &str = r"\[(.*?)\]";
const PARAMETER_SEPARATOR: char = ',';

// AzureAIStudioConfig's retry policy: a truthy per-call drop_params or the proxy-wide litellm.drop_params
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoundryRepair {
    pub call: bool,
    pub global: bool,
}

impl FoundryRepair {
    #[must_use]
    pub fn should_retry(&self, error_text: &str) -> bool {
        if error_text.contains(EXTRA_INPUTS_MARKER)
            && (self.call
                || self.global
                || Regex::new(TOOL_INDEX_PATTERN)
                    .ok()
                    .is_some_and(|pattern| pattern.is_match(error_text)))
        {
            return true;
        }
        // BaseConfig, the superclass LiteLLM falls back to here, answers False for every other text
        error_text.contains(INVALID_TOOL_CALL_INDEX_MARKER)
            || error_text.contains(PASS_THROUGH_HINT)
    }
}

#[derive(Debug, Error)]
pub enum RepairError {
    #[error(transparent)]
    Rejected(#[from] RejectedFieldsError),
    #[error("the chat messages cannot be read: {source}")]
    Messages {
        #[from]
        source: serde_json::Error,
    },
}

// LiteLLM's transform_request_on_unprocessable_entity_error: the three repairs run in its order, then B505 drops the loc fields
pub fn repair(error_text: &str, request: &Object) -> Result<Object, RepairError> {
    let mut repaired = request.clone();
    let takes_index_branch =
        error_text.contains(INVALID_TOOL_CALL_INDEX_MARKER) && has_messages(&repaired);
    if takes_index_branch {
        remove_tool_call_indexes(&mut repaired)?;
    }
    if !takes_index_branch && error_text.contains(PASS_THROUGH_HINT) {
        drop_extra_parameters(&mut repaired, error_text);
    }
    if error_text.contains(EXTRA_INPUTS_MARKER)
        && Regex::new(TOOL_INDEX_PATTERN)
            .ok()
            .is_some_and(|pattern| pattern.is_match(error_text))
    {
        drop_tool_level_extra_fields(&mut repaired, error_text);
    }
    let kept = without_rejected_fields(ErrorBody::Http(error_text.as_bytes()), &repaired)?;
    Ok(kept)
}

// request_data.get("messages") is not None: an absent key and a null value both skip the tool-call repair
fn has_messages(request: &Object) -> bool {
    request
        .get("messages")
        .is_some_and(|messages| !messages.is_null())
}

fn remove_tool_call_indexes(request: &mut Object) -> Result<(), RepairError> {
    let Some(messages) = request.get_mut("messages") else {
        return Ok(());
    };
    let mut parsed: Vec<IndexMap<String, Json>> = serde_json::from_value(messages.clone())?;
    remove_index_from_tool_calls(&mut parsed);
    *messages = serde_json::to_value(parsed)?;
    Ok(())
}

fn drop_extra_parameters(request: &mut Object, error_text: &str) {
    for name in extra_parameters(error_text) {
        request.remove(name.as_str());
    }
}

// Python's str.strip() removes U+001C..U+001F too, which char::is_whitespace leaves
fn strip_parameter(parameter: &str) -> &str {
    parameter
        .trim_matches(|character: char| {
            character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
        })
        .trim_matches('\'')
        .trim_matches('"')
}

// LiteLLM's _extract_params_to_drop_from_error_text: the first bracketed run, split on commas, stripped of whitespace then quotes
fn extra_parameters(error_text: &str) -> Vec<String> {
    let Some(group) = first_bracket_group(error_text) else {
        return Vec::new();
    };
    group
        .split(PARAMETER_SEPARATOR)
        .map(strip_parameter)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

// re's `\[(.*?)\]` searched left to right: the dot stops at a newline, so an unclosed bracket is skipped and a later one can match
fn first_bracket_group(error_text: &str) -> Option<&str> {
    let pattern = Regex::new(BRACKET_PATTERN).ok()?;
    let found = pattern.captures(error_text)?;
    let group = found.get(1)?;
    Some(group.as_str())
}

fn drop_tool_level_extra_fields(request: &mut Object, error_text: &str) {
    let names = tool_level_field_names(error_text);
    let Some(tools) = request
        .get_mut("tools")
        .and_then(|value| value.as_array_mut())
    else {
        return;
    };
    for tool in tools {
        let Some(fields) = tool.as_object_mut() else {
            continue;
        };
        for name in &names {
            fields.remove(name.as_str());
        }
    }
}

// re.findall(r"tools\[\d+\]\.([\w-]+)"): every non-overlapping field name, the same one only once
fn tool_level_field_names(error_text: &str) -> Vec<String> {
    let Some(pattern) = Regex::new(TOOL_FIELD_PATTERN).ok() else {
        return Vec::new();
    };
    let mut names: Vec<&str> = Vec::new();
    for found in pattern.captures_iter(error_text) {
        let Some(name) = found.get(1) else {
            continue;
        };
        let text = name.as_str();
        if !names.contains(&text) {
            names.push(text);
        }
    }
    names.into_iter().map(str::to_owned).collect()
}

mod tests;

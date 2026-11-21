use betterlite_models::call_helpers::tool_calls::remove_index_from_tool_calls;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::output_limits::{ErrorBody, RejectedFieldsError, without_rejected_fields};
use indexmap::IndexMap;
use thiserror::Error;
use unicode_general_category::{GeneralCategory, get_general_category};

// LiteLLM's AzureFoundryErrorStrings.SET_EXTRA_PARAMETERS_TO_PASS_THROUGH
pub const PASS_THROUGH_HINT: &str = "Set extra-parameters to 'pass-through'";
// the first marker should_retry_llm_api_inside_llm_translation_on_http_error matches
pub const EXTRA_INPUTS_MARKER: &str = "Extra inputs are not permitted";
// the second marker the same policy always retries on
pub const INVALID_TOOL_CALL_INDEX_MARKER: &str =
    "unknown field: parameter index is not a valid field";
// max_retry_on_unprocessable_entity_error: LiteLLM's range(max(2, 1)) makes two sends, so one repair
pub const MAX_ATTEMPTS: u32 = 2;

const TOOL_INDEX_PREFIX: &str = "tools[";
const TOOL_INDEX_SUFFIX: &str = "].";
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
            && (self.call || self.global || has_tool_level_extra_fields(error_text))
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
    if error_text.contains(INVALID_TOOL_CALL_INDEX_MARKER) && has_messages(&repaired) {
        remove_tool_call_indexes(&mut repaired)?;
    } else if error_text.contains(PASS_THROUGH_HINT) {
        drop_extra_parameters(&mut repaired, error_text);
    }
    if error_text.contains(EXTRA_INPUTS_MARKER) && has_tool_level_extra_fields(error_text) {
        drop_tool_level_extra_fields(&mut repaired, error_text);
    }
    Ok(without_rejected_fields(
        ErrorBody::Http(error_text.as_bytes()),
        &repaired,
    )?)
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

// LiteLLM's _extract_params_to_drop_from_error_text: the first bracketed run, split on commas, stripped of whitespace then quotes
fn extra_parameters(error_text: &str) -> Vec<String> {
    let Some(group) = first_bracket_group(error_text) else {
        return Vec::new();
    };
    group
        .split(PARAMETER_SEPARATOR)
        .map(|parameter| {
            parameter
                .trim_matches(is_python_whitespace)
                .trim_matches('\'')
                .trim_matches('"')
        })
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

// Python's str.strip() reads the White_Space property and the four separators below it as whitespace
const fn is_python_whitespace(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
}

// re's `\[(.*?)\]` searched left to right: the dot stops at a newline, so an unclosed bracket is skipped and a later one can match
fn first_bracket_group(error_text: &str) -> Option<&str> {
    let mut from = 0_usize;
    while let Some(offset) = error_text.get(from..).and_then(|rest| rest.find('[')) {
        let open = from.saturating_add(offset);
        let rest = error_text.get(open.saturating_add(1)..)?;
        let line_end = rest.find('\n').unwrap_or(rest.len());
        let line = rest.get(..line_end)?;
        if let Some(close) = line.find(']') {
            return line.get(..close);
        }
        from = open.saturating_add(1);
    }
    None
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

// LiteLLM's _error_has_tool_level_extra_fields: re.search(r"tools\[\d+\]\.")
fn has_tool_level_extra_fields(error_text: &str) -> bool {
    let mut from = 0_usize;
    while let Some(offset) = error_text
        .get(from..)
        .and_then(|rest| rest.find(TOOL_INDEX_PREFIX))
    {
        let start = from.saturating_add(offset);
        if tool_index_end(error_text, start).is_some() {
            return true;
        }
        from = start.saturating_add(1);
    }
    false
}

// re.findall(r"tools\[\d+\]\.([\w-]+)"): every non-overlapping field name, the same one only once
fn tool_level_field_names(error_text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut from = 0_usize;
    while let Some(offset) = error_text
        .get(from..)
        .and_then(|rest| rest.find(TOOL_INDEX_PREFIX))
    {
        let start = from.saturating_add(offset);
        let Some(after_index) = tool_index_end(error_text, start) else {
            from = start.saturating_add(1);
            continue;
        };
        let rest = error_text.get(after_index..).unwrap_or_default();
        let name: String = rest
            .chars()
            .take_while(|character| is_tool_field_char(*character))
            .collect();
        if name.is_empty() {
            from = start.saturating_add(1);
            continue;
        }
        from = after_index.saturating_add(name.len());
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

// Python's \d is the Unicode decimal-digit category
fn tool_index_end(error_text: &str, start: usize) -> Option<usize> {
    let digits_start = start.saturating_add(TOOL_INDEX_PREFIX.len());
    let rest = error_text.get(digits_start..)?;
    let digits: usize = rest
        .chars()
        .take_while(|character| get_general_category(*character) == GeneralCategory::DecimalNumber)
        .map(char::len_utf8)
        .sum();
    if digits == 0 {
        return None;
    }
    let after_digits = digits_start.saturating_add(digits);
    if !error_text
        .get(after_digits..)?
        .starts_with(TOOL_INDEX_SUFFIX)
    {
        return None;
    }
    Some(after_digits.saturating_add(TOOL_INDEX_SUFFIX.len()))
}

// Python's [\w-]: str.isalnum()'s letter and number categories, the underscore and the hyphen
fn is_tool_field_char(character: char) -> bool {
    character == '_'
        || character == '-'
        || matches!(
            get_general_category(character),
            GeneralCategory::UppercaseLetter
                | GeneralCategory::LowercaseLetter
                | GeneralCategory::TitlecaseLetter
                | GeneralCategory::ModifierLetter
                | GeneralCategory::OtherLetter
                | GeneralCategory::DecimalNumber
                | GeneralCategory::LetterNumber
                | GeneralCategory::OtherNumber
        )
}

mod tests;

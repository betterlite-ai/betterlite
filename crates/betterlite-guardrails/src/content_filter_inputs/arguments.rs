use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::wire::parse_request_data;
use betterlite_logging::analytics::python::float_text;

use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_actions::{ContentFilterSource, filter_single_text};

use super::error::ArgumentFilterError;

pub const TOOL_CALL_LABEL: &str = "tool call";

// serde_json parses 127 containers and the parse wrapper takes one, so arguments this deep never reach the tree filter
const PARSE_DEPTH_LIMIT: i64 = 127;

#[derive(Debug)]
pub struct ArgumentFilter<'source> {
    pub source: &'source ContentFilterSource<'source>,
    pub max_depth: i64,
}

pub fn filter_argument_value(
    filter: &ArgumentFilter<'_>,
    value: &GuardrailValue,
    detections: &mut Vec<ContentFilterDetection>,
    context_label: &'static str,
    depth: i64,
) -> Result<GuardrailValue, ArgumentFilterError> {
    if depth > filter.max_depth {
        return Err(ArgumentFilterError::Depth { context_label });
    }
    match value {
        GuardrailValue::Text(text) => {
            filter_text(filter.source, text, detections).map(GuardrailValue::Text)
        }
        GuardrailValue::SignedInteger(number) => {
            assert_argument_label_clean(filter, &number.to_string(), detections, context_label)?;
            Ok(value.clone())
        }
        GuardrailValue::UnsignedInteger(number) => {
            assert_argument_label_clean(filter, &number.to_string(), detections, context_label)?;
            Ok(value.clone())
        }
        GuardrailValue::Decimal(number) => {
            assert_argument_label_clean(filter, &float_text(*number), detections, context_label)?;
            Ok(value.clone())
        }
        GuardrailValue::Object(entries) => {
            filter_object(filter, entries, detections, context_label, depth)
                .map(GuardrailValue::Object)
        }
        GuardrailValue::Array(items) => {
            let next = depth.saturating_add(1);
            let filtered = items
                .iter()
                .map(|item| filter_argument_value(filter, item, detections, context_label, next))
                .collect::<Result<Vec<_>, ArgumentFilterError>>()?;
            Ok(GuardrailValue::Array(filtered))
        }
        GuardrailValue::Null | GuardrailValue::Boolean(_) => Ok(value.clone()),
    }
}

pub fn scan_tool_call_arguments(
    filter: &ArgumentFilter<'_>,
    tool_calls: &mut [GuardrailValue],
    detections: &mut Vec<ContentFilterDetection>,
) -> Result<(), ArgumentFilterError> {
    for tool_call in tool_calls.iter_mut() {
        let Some(arguments) = function_arguments(tool_call) else {
            continue;
        };
        let filtered = filter_tool_call_arguments(filter, arguments, detections)?;
        if filtered != arguments {
            set_function_arguments(tool_call, filtered);
        }
    }
    Ok(())
}

pub fn filter_tool_call_arguments(
    filter: &ArgumentFilter<'_>,
    arguments: &str,
    detections: &mut Vec<ContentFilterDetection>,
) -> Result<String, ArgumentFilterError> {
    // json.loads restores every number; arbitrary_precision parks floats and wide integers in a private map instead
    let wrapped = format!("{{\"arguments\":{arguments}}}");
    let Some(parsed) = parse_request_data(&wrapped)
        .ok()
        .and_then(|mut entries| entries.remove("arguments"))
    else {
        if nesting_past_the_parse_limit(arguments, filter.max_depth) {
            return Err(ArgumentFilterError::Depth {
                context_label: TOOL_CALL_LABEL,
            });
        }
        return filter_text(filter.source, arguments, detections);
    };
    if !matches!(parsed, GuardrailValue::Object(_) | GuardrailValue::Array(_)) {
        return filter_text(filter.source, arguments, detections);
    }
    let filtered = filter_argument_value(filter, &parsed, detections, TOOL_CALL_LABEL, 0)?;
    if filtered == parsed {
        return Ok(arguments.to_owned());
    }
    serde_json::to_string(&filtered).map_err(|source| ArgumentFilterError::Serialize { source })
}

// Past serde_json's parse limit the tree never forms, so the depth check reads the raw nesting instead
fn nesting_past_the_parse_limit(source: &str, max_depth: i64) -> bool {
    let (mut open, mut deepest, mut nesting) = (0_i64, 0_i64, 0_i64);
    let (mut in_string, mut escaped) = (false, false);
    for byte in source.bytes() {
        match (escaped, in_string, byte) {
            (true, _, _) => escaped = false,
            (false, true, b'\\') => escaped = true,
            (false, true, b'"') => in_string = false,
            (false, false, b'"') => {
                in_string = true;
                deepest = deepest.max(open);
            }
            (false, false, b'{' | b'[') => {
                deepest = deepest.max(open);
                open = open.saturating_add(1);
                nesting = nesting.max(open);
            }
            (false, false, b'}' | b']') => open = open.saturating_sub(1),
            (false, false, _) if byte.is_ascii_alphanumeric() || byte == b'-' => {
                deepest = deepest.max(open);
            }
            (false, _, _) => {}
        }
    }
    nesting >= PARSE_DEPTH_LIMIT && deepest > max_depth
}

fn filter_object(
    filter: &ArgumentFilter<'_>,
    entries: &BTreeMap<String, GuardrailValue>,
    detections: &mut Vec<ContentFilterDetection>,
    context_label: &'static str,
    depth: i64,
) -> Result<BTreeMap<String, GuardrailValue>, ArgumentFilterError> {
    for key in entries.keys() {
        assert_argument_label_clean(filter, key, detections, context_label)?;
    }
    let mut filtered = BTreeMap::new();
    let next = depth.saturating_add(1);
    for (key, item) in entries {
        let value = filter_argument_value(filter, item, detections, context_label, next)?;
        filtered.insert(key.clone(), value);
    }
    Ok(filtered)
}

fn assert_argument_label_clean(
    filter: &ArgumentFilter<'_>,
    text: &str,
    detections: &mut Vec<ContentFilterDetection>,
    context_label: &'static str,
) -> Result<(), ArgumentFilterError> {
    if filter_text(filter.source, text, detections)? == text {
        return Ok(());
    }
    Err(ArgumentFilterError::Label { context_label })
}

fn filter_text(
    source: &ContentFilterSource<'_>,
    text: &str,
    detections: &mut Vec<ContentFilterDetection>,
) -> Result<String, ArgumentFilterError> {
    match filter_single_text(source, text, Some(detections)) {
        Ok(filtered) => Ok(filtered),
        Err(ContentFilterActionError::Blocked { detail }) => {
            Err(ArgumentFilterError::Blocked { detail })
        }
        Err(error) => Err(ArgumentFilterError::Filter(error)),
    }
}

fn function_arguments(tool_call: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Object(entries) = tool_call else {
        return None;
    };
    let GuardrailValue::Object(function) = entries.get("function")? else {
        return None;
    };
    let GuardrailValue::Text(arguments) = function.get("arguments")? else {
        return None;
    };
    (!arguments.trim().is_empty()).then_some(arguments.as_str())
}

fn set_function_arguments(tool_call: &mut GuardrailValue, arguments: String) {
    let GuardrailValue::Object(entries) = tool_call else {
        return;
    };
    let Some(GuardrailValue::Object(function)) = entries.get_mut("function") else {
        return;
    };
    function.insert("arguments".to_owned(), GuardrailValue::Text(arguments));
}

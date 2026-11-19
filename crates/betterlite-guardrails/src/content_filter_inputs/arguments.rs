use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_actions::{ContentFilterSource, filter_single_text};

use super::error::ArgumentFilterError;

pub const TOOL_CALL_LABEL: &str = "tool call";

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
        GuardrailValue::Text(text) => Ok(GuardrailValue::Text(filter_text(
            filter.source,
            text,
            detections,
        )?)),
        GuardrailValue::SignedInteger(number) => {
            assert_argument_label_clean(filter, &number.to_string(), detections, context_label)?;
            Ok(value.clone())
        }
        GuardrailValue::UnsignedInteger(number) => {
            assert_argument_label_clean(filter, &number.to_string(), detections, context_label)?;
            Ok(value.clone())
        }
        GuardrailValue::Decimal(number) => {
            assert_argument_label_clean(filter, &number.to_string(), detections, context_label)?;
            Ok(value.clone())
        }
        GuardrailValue::Object(entries) => {
            filter_object(filter, entries, detections, context_label, depth)
        }
        GuardrailValue::Array(items) => {
            filter_array(filter, items, detections, context_label, depth)
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
    let Ok(parsed) = serde_json::from_str::<GuardrailValue>(arguments) else {
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

fn filter_object(
    filter: &ArgumentFilter<'_>,
    entries: &BTreeMap<String, GuardrailValue>,
    detections: &mut Vec<ContentFilterDetection>,
    context_label: &'static str,
    depth: i64,
) -> Result<GuardrailValue, ArgumentFilterError> {
    for key in entries.keys() {
        assert_argument_label_clean(filter, key, detections, context_label)?;
    }
    let mut filtered = BTreeMap::new();
    for (key, item) in entries {
        filtered.insert(
            key.clone(),
            filter_argument_value(filter, item, detections, context_label, depth + 1)?,
        );
    }
    Ok(GuardrailValue::Object(filtered))
}

fn filter_array(
    filter: &ArgumentFilter<'_>,
    items: &[GuardrailValue],
    detections: &mut Vec<ContentFilterDetection>,
    context_label: &'static str,
    depth: i64,
) -> Result<GuardrailValue, ArgumentFilterError> {
    let mut filtered = Vec::with_capacity(items.len());
    for item in items {
        filtered.push(filter_argument_value(
            filter,
            item,
            detections,
            context_label,
            depth + 1,
        )?);
    }
    Ok(GuardrailValue::Array(filtered))
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

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::ResponseKind;
use crate::cato::error::CatoError;

// one step of the route to a fragment inside the response body
#[derive(Debug, Clone, Copy)]
pub(crate) enum Step {
    Key(&'static str),
    Index(usize),
}

// an assistant text fragment and where its redaction has to be written back
#[derive(Debug)]
pub(crate) struct Fragment {
    pub(crate) path: Vec<Step>,
    pub(crate) text: String,
}

// LiteLLM reads assistant fragments from choices[] or from output[], depending on the response shape
pub(crate) fn response_fragments(response: &GuardrailValue, kind: ResponseKind) -> Vec<Fragment> {
    let (field, fragments_of): (&str, fn(&GuardrailValue, usize) -> Vec<Fragment>) = match kind {
        ResponseKind::Chat => ("choices", message_fragments),
        ResponseKind::Responses => ("output", output_item_fragments),
    };
    let Some(items) = field_of(response, field).and_then(as_array) else {
        return Vec::new();
    };
    let mut fragments = Vec::new();
    for (index, item) in items.iter().enumerate() {
        fragments.extend(fragments_of(item, index));
    }
    fragments
}

// content is a fragment whenever it is present, and then every tool call's non-empty arguments
fn message_fragments(choice: &GuardrailValue, choice_index: usize) -> Vec<Fragment> {
    let Some(message) = field_of(choice, "message") else {
        return Vec::new();
    };
    let mut fragments = Vec::new();
    if let Some(text) = field_of(message, "content").and_then(as_text) {
        fragments.push(Fragment {
            path: vec![
                Step::Key("choices"),
                Step::Index(choice_index),
                Step::Key("message"),
                Step::Key("content"),
            ],
            text: text.to_owned(),
        });
    }
    fragments.extend(tool_call_fragments(message, choice_index));
    fragments
}

// every tool call's non-empty arguments string, written back at its own index
fn tool_call_fragments(message: &GuardrailValue, choice_index: usize) -> Vec<Fragment> {
    let Some(tool_calls) = field_of(message, "tool_calls").and_then(as_array) else {
        return Vec::new();
    };
    let mut fragments = Vec::new();
    for (call_index, tool_call) in tool_calls.iter().enumerate() {
        let arguments = field_of(tool_call, "function")
            .and_then(|function| field_of(function, "arguments"))
            .and_then(as_text)
            .filter(|text| !text.is_empty());
        let Some(text) = arguments else {
            continue;
        };
        fragments.push(Fragment {
            path: vec![
                Step::Key("choices"),
                Step::Index(choice_index),
                Step::Key("message"),
                Step::Key("tool_calls"),
                Step::Index(call_index),
                Step::Key("function"),
                Step::Key("arguments"),
            ],
            text: text.to_owned(),
        });
    }
    fragments
}

// LiteLLM's _responses_output_fragments: function-call arguments and every output_text block of a message
fn output_item_fragments(item: &GuardrailValue, item_index: usize) -> Vec<Fragment> {
    match field_of(item, "type").and_then(as_text) {
        Some("function_call") => function_call_fragment(item, item_index)
            .into_iter()
            .collect(),
        Some("message") => output_text_fragments(item, item_index),
        _ => Vec::new(),
    }
}

fn function_call_fragment(item: &GuardrailValue, item_index: usize) -> Option<Fragment> {
    let text = field_of(item, "arguments")
        .and_then(as_text)
        .filter(|text| !text.is_empty())?;
    Some(Fragment {
        path: vec![
            Step::Key("output"),
            Step::Index(item_index),
            Step::Key("arguments"),
        ],
        text: text.to_owned(),
    })
}

fn output_text_fragments(item: &GuardrailValue, item_index: usize) -> Vec<Fragment> {
    let Some(parts) = field_of(item, "content").and_then(as_array) else {
        return Vec::new();
    };
    let mut fragments = Vec::new();
    for (part_index, part) in parts.iter().enumerate() {
        if field_of(part, "type").and_then(as_text) != Some("output_text") {
            continue;
        }
        let text = field_of(part, "text")
            .and_then(as_text)
            .filter(|text| !text.is_empty());
        let Some(text) = text else {
            continue;
        };
        fragments.push(Fragment {
            path: vec![
                Step::Key("output"),
                Step::Index(item_index),
                Step::Key("content"),
                Step::Index(part_index),
                Step::Key("text"),
            ],
            text: text.to_owned(),
        });
    }
    fragments
}

// LiteLLM's _apply_output_fragment and _apply_responses_output_fragment write into the fragment they read
pub(crate) fn write_fragment(
    response: &mut GuardrailValue,
    path: &[Step],
    redacted: String,
) -> Result<(), CatoError> {
    let Some((last, parents)) = path.split_last() else {
        return Err(CatoError::OutputRedactionMismatch);
    };
    let mut current = response;
    for step in parents {
        current = match (step, current) {
            (Step::Key(key), GuardrailValue::Object(object)) => object.get_mut(*key),
            (Step::Index(index), GuardrailValue::Array(items)) => items.get_mut(*index),
            _ => None,
        }
        .ok_or(CatoError::OutputRedactionMismatch)?;
    }
    let Step::Key(key) = last else {
        return Err(CatoError::OutputRedactionMismatch);
    };
    let GuardrailValue::Object(object) = current else {
        return Err(CatoError::OutputRedactionMismatch);
    };
    object.insert((*key).to_owned(), GuardrailValue::Text(redacted));
    Ok(())
}

fn field_of<'value>(value: &'value GuardrailValue, key: &str) -> Option<&'value GuardrailValue> {
    match value {
        GuardrailValue::Object(object) => object.get(key),
        _ => None,
    }
}

fn as_array(value: &GuardrailValue) -> Option<&[GuardrailValue]> {
    match value {
        GuardrailValue::Array(items) => Some(items),
        _ => None,
    }
}

fn as_text(value: &GuardrailValue) -> Option<&str> {
    match value {
        GuardrailValue::Text(text) => Some(text),
        _ => None,
    }
}

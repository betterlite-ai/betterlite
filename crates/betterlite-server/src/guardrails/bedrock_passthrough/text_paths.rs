use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub mod steps;
mod tests;

use steps::{PathStep, TextPath, child_path, text_at_path_mut};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkipFlags {
    pub system: bool,
    pub tool: bool,
}

#[must_use]
pub fn collect_request_texts(body: &GuardrailValue, skip: SkipFlags) -> Vec<(TextPath, String)> {
    let GuardrailValue::Object(fields) = body else {
        return Vec::new();
    };
    let mut holders = Vec::new();
    if !skip.system {
        collect_system_texts(fields, &mut holders);
    }
    collect_message_texts(fields, skip.tool, &mut holders);
    let additional_path = [PathStep::Key("additionalModelRequestFields".to_owned())];
    collect_strings(
        fields.get("additionalModelRequestFields"),
        &additional_path,
        &mut holders,
    );
    holders
}

#[must_use]
pub fn collect_output_texts(content_blocks: &[GuardrailValue]) -> Vec<(TextPath, String)> {
    let mut holders = Vec::new();
    for (index, block) in content_blocks.iter().enumerate() {
        collect_output_block(index, block, &mut holders);
    }
    holders
}

pub fn write_back_texts(root: &mut GuardrailValue, holders: &[TextPath], returned: &[String]) {
    if returned.len() < holders.len() {
        tracing::warn!(
            "BedrockPassthroughGuardrailHandler: guardrail returned {} texts for {} extracted fields; the unreturned fields keep their original text",
            returned.len(),
            holders.len()
        );
    }
    for (path, text) in holders.iter().zip(returned) {
        if let Some(target) = text_at_path_mut(root, &path.0) {
            target.clone_from(text);
        }
    }
}

fn collect_system_texts(
    fields: &BTreeMap<String, GuardrailValue>,
    holders: &mut Vec<(TextPath, String)>,
) {
    let Some(GuardrailValue::Array(blocks)) = fields.get("system") else {
        return;
    };
    let base = [PathStep::Key("system".to_owned())];
    collect_block_texts(blocks, &base, holders);
}

fn collect_block_texts(
    blocks: &[GuardrailValue],
    base: &[PathStep],
    holders: &mut Vec<(TextPath, String)>,
) {
    for (index, entry) in blocks.iter().enumerate() {
        let GuardrailValue::Object(block) = entry else {
            continue;
        };
        let mut path = base.to_vec();
        path.push(PathStep::Index(index));
        push_block_text(block, &path, holders);
    }
}

fn collect_message_texts(
    fields: &BTreeMap<String, GuardrailValue>,
    skip_tool: bool,
    holders: &mut Vec<(TextPath, String)>,
) {
    let Some(GuardrailValue::Array(messages)) = fields.get("messages") else {
        return;
    };
    for (message_index, entry) in messages.iter().enumerate() {
        let GuardrailValue::Object(message) = entry else {
            continue;
        };
        let Some(GuardrailValue::Array(blocks)) = message.get("content") else {
            continue;
        };
        for (block_index, block) in blocks.iter().enumerate() {
            let block_path = [
                PathStep::Key("messages".to_owned()),
                PathStep::Index(message_index),
                PathStep::Key("content".to_owned()),
                PathStep::Index(block_index),
            ];
            collect_message_block(&block_path, block, skip_tool, holders);
        }
    }
}

fn collect_message_block(
    block_path: &[PathStep],
    block_value: &GuardrailValue,
    skip_tool: bool,
    holders: &mut Vec<(TextPath, String)>,
) {
    let GuardrailValue::Object(block) = block_value else {
        return;
    };
    if skip_tool && (block.contains_key("toolUse") || block.contains_key("toolResult")) {
        return;
    }
    push_block_text(block, block_path, holders);
    if let Some(GuardrailValue::Object(tool_use)) = block.get("toolUse") {
        collect_strings(
            tool_use.get("input"),
            &child_path(block_path, &["toolUse", "input"]),
            holders,
        );
    }
    if let Some(GuardrailValue::Object(tool_result)) = block.get("toolResult") {
        collect_tool_result_texts(block_path, tool_result, holders);
    }
}

fn collect_tool_result_texts(
    block_path: &[PathStep],
    tool_result: &BTreeMap<String, GuardrailValue>,
    holders: &mut Vec<(TextPath, String)>,
) {
    let Some(GuardrailValue::Array(results)) = tool_result.get("content") else {
        return;
    };
    let content_path = child_path(block_path, &["toolResult", "content"]);
    for (index, entry) in results.iter().enumerate() {
        let GuardrailValue::Object(result) = entry else {
            continue;
        };
        let mut result_path = content_path.clone();
        result_path.push(PathStep::Index(index));
        push_block_text(result, &result_path, holders);
        collect_strings(
            result.get("json"),
            &child_path(&result_path, &["json"]),
            holders,
        );
    }
}

fn collect_output_block(
    index: usize,
    block_value: &GuardrailValue,
    holders: &mut Vec<(TextPath, String)>,
) {
    let GuardrailValue::Object(block) = block_value else {
        return;
    };
    let base = [PathStep::Index(index)];
    push_block_text(block, &base, holders);
    if let Some(GuardrailValue::Object(tool_use)) = block.get("toolUse") {
        collect_strings(
            tool_use.get("input"),
            &child_path(&base, &["toolUse", "input"]),
            holders,
        );
    }
    if let Some(GuardrailValue::Object(reasoning)) = block.get("reasoningContent")
        && let Some(GuardrailValue::Object(reasoning_text)) = reasoning.get("reasoningText")
    {
        push_block_text(
            reasoning_text,
            &child_path(&base, &["reasoningContent", "reasoningText"]),
            holders,
        );
    }
    let Some(GuardrailValue::Object(citations)) = block.get("citationsContent") else {
        return;
    };
    let Some(GuardrailValue::Array(cited_blocks)) = citations.get("content") else {
        return;
    };
    let content_path = child_path(&base, &["citationsContent", "content"]);
    for (cited_index, entry) in cited_blocks.iter().enumerate() {
        let GuardrailValue::Object(cited) = entry else {
            continue;
        };
        let mut cited_path = content_path.clone();
        cited_path.push(PathStep::Index(cited_index));
        push_block_text(cited, &cited_path, holders);
    }
}

// LiteLLM's _collect_strings: direct string children first, then the containers popped last in, first out.
fn collect_strings(
    node: Option<&GuardrailValue>,
    base: &[PathStep],
    holders: &mut Vec<(TextPath, String)>,
) {
    let Some(root) = node else {
        return;
    };
    let mut stack: Vec<(&GuardrailValue, Vec<PathStep>)> = vec![(root, base.to_vec())];
    while let Some((current, path)) = stack.pop() {
        match current {
            GuardrailValue::Object(fields) => {
                let children = fields
                    .iter()
                    .map(|(key, value)| (PathStep::Key(key.clone()), value));
                collect_child_strings(children, &path, &mut stack, holders);
            }
            GuardrailValue::Array(items) => {
                let children = items
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (PathStep::Index(index), value));
                collect_child_strings(children, &path, &mut stack, holders);
            }
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_) => {}
        }
    }
}

fn collect_child_strings<'value, Children>(
    children: Children,
    path: &[PathStep],
    stack: &mut Vec<(&'value GuardrailValue, Vec<PathStep>)>,
    holders: &mut Vec<(TextPath, String)>,
) where
    Children: Iterator<Item = (PathStep, &'value GuardrailValue)>,
{
    for (step, value) in children {
        let mut child = path.to_vec();
        child.push(step);
        match value {
            GuardrailValue::Object(_) | GuardrailValue::Array(_) => stack.push((value, child)),
            GuardrailValue::Text(text) => {
                if !text.is_empty() {
                    holders.push((TextPath(child), text.clone()));
                }
            }
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_) => {}
        }
    }
}

fn push_block_text(
    block: &BTreeMap<String, GuardrailValue>,
    base: &[PathStep],
    holders: &mut Vec<(TextPath, String)>,
) {
    let Some(GuardrailValue::Text(text)) = block.get("text") else {
        return;
    };
    if text.is_empty() {
        return;
    }
    holders.push((TextPath(child_path(base, &["text"])), text.clone()));
}

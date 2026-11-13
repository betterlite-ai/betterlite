use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::inspection::synthetic_message;
use super::wire::RedactedMessage;

const SCALAR_KEYS: [&str; 4] = ["description", "title", "const", "default"];
const LIST_KEYS: [&str; 2] = ["enum", "examples"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaRoot {
    Tool(usize),
    Function(usize),
    ResponseFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaStep {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaSlot {
    Field {
        root: SchemaRoot,
        path: Vec<SchemaStep>,
        key: String,
    },
    Item {
        root: SchemaRoot,
        path: Vec<SchemaStep>,
        key: String,
        index: usize,
    },
}

pub(crate) fn schema_string_refs(
    data: &BTreeMap<String, GuardrailValue>,
) -> Vec<(SchemaSlot, String)> {
    let mut refs = Vec::new();
    let mut stack = roots(data)
        .into_iter()
        .rev()
        .map(|(root, value)| (root, Vec::new(), value))
        .collect::<Vec<_>>();
    while let Some((root, path, node)) = stack.pop() {
        match node {
            GuardrailValue::Object(object) => {
                for key in SCALAR_KEYS {
                    if let Some(GuardrailValue::Text(text)) = object.get(key)
                        && !text.is_empty()
                    {
                        refs.push((
                            SchemaSlot::Field {
                                root: root.clone(),
                                path: path.clone(),
                                key: key.to_owned(),
                            },
                            text.clone(),
                        ));
                    }
                }
                for key in LIST_KEYS {
                    if let Some(GuardrailValue::Array(items)) = object.get(key) {
                        for (index, item) in items.iter().enumerate() {
                            if let GuardrailValue::Text(text) = item
                                && !text.is_empty()
                            {
                                refs.push((
                                    SchemaSlot::Item {
                                        root: root.clone(),
                                        path: path.clone(),
                                        key: key.to_owned(),
                                        index,
                                    },
                                    text.clone(),
                                ));
                            }
                        }
                    }
                }
                for (key, value) in object.iter().rev() {
                    let mut child = path.clone();
                    child.push(SchemaStep::Key(key.clone()));
                    stack.push((root.clone(), child, value));
                }
            }
            GuardrailValue::Array(items) => {
                for (index, item) in items.iter().enumerate().rev() {
                    let mut child = path.clone();
                    child.push(SchemaStep::Index(index));
                    stack.push((root.clone(), child, item));
                }
            }
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_) => {}
        }
    }
    refs
}

#[must_use]
pub fn schema_string_messages(data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    schema_string_refs(data)
        .into_iter()
        .map(|(_slot, text)| synthetic_message("system", &text))
        .collect()
}

pub(crate) fn apply_schema_string_redaction(
    data: &mut BTreeMap<String, GuardrailValue>,
    redacted: &[RedactedMessage],
) {
    let slots = schema_string_refs(data);
    let mut replacements = redacted.iter();
    for (slot, _text) in slots {
        let Some(message) = replacements.next() else {
            break;
        };
        let Some(replacement) = &message.content else {
            continue;
        };
        let Some(node) = slot_node_mut(data, &slot) else {
            continue;
        };
        match &slot {
            SchemaSlot::Field { key, .. } => {
                if let GuardrailValue::Object(object) = node {
                    object.insert(key.clone(), replacement.clone());
                }
            }
            SchemaSlot::Item { key, index, .. } => {
                if let GuardrailValue::Object(object) = node
                    && let Some(GuardrailValue::Array(items)) = object.get_mut(key)
                    && let Some(item) = items.get_mut(*index)
                {
                    *item = replacement.clone();
                }
            }
        }
    }
}

fn roots(data: &BTreeMap<String, GuardrailValue>) -> Vec<(SchemaRoot, &GuardrailValue)> {
    let mut roots = Vec::new();
    if let Some(GuardrailValue::Array(tools)) = data.get("tools") {
        for (index, tool) in tools.iter().enumerate() {
            if let GuardrailValue::Object(object) = tool
                && let Some(function @ GuardrailValue::Object(_)) = object.get("function")
            {
                roots.push((SchemaRoot::Tool(index), function));
            }
        }
    }
    if let Some(GuardrailValue::Array(functions)) = data.get("functions") {
        for (index, function) in functions.iter().enumerate() {
            if matches!(function, GuardrailValue::Object(_)) {
                roots.push((SchemaRoot::Function(index), function));
            }
        }
    }
    if let Some(response_format @ GuardrailValue::Object(_)) = data.get("response_format") {
        roots.push((SchemaRoot::ResponseFormat, response_format));
    }
    roots
}

fn slot_node_mut<'a>(
    data: &'a mut BTreeMap<String, GuardrailValue>,
    slot: &SchemaSlot,
) -> Option<&'a mut GuardrailValue> {
    let (root, path) = match slot {
        SchemaSlot::Field { root, path, .. } | SchemaSlot::Item { root, path, .. } => (root, path),
    };
    let mut current = match root {
        SchemaRoot::Tool(index) => match data.get_mut("tools") {
            Some(GuardrailValue::Array(tools)) => match tools.get_mut(*index) {
                Some(GuardrailValue::Object(object)) => object.get_mut("function"),
                _ => None,
            },
            _ => None,
        },
        SchemaRoot::Function(index) => match data.get_mut("functions") {
            Some(GuardrailValue::Array(functions)) => functions.get_mut(*index),
            _ => None,
        },
        SchemaRoot::ResponseFormat => data.get_mut("response_format"),
    }?;
    for step in path {
        current = match (step, current) {
            (SchemaStep::Key(key), GuardrailValue::Object(object)) => object.get_mut(key)?,
            (SchemaStep::Index(index), GuardrailValue::Array(items)) => items.get_mut(*index)?,
            (SchemaStep::Key(_) | SchemaStep::Index(_), _) => return None,
        };
    }
    Some(current)
}

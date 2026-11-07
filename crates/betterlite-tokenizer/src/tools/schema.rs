use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

use crate::tools::error::ToolFormatError;

#[derive(Debug, Deserialize)]
pub(super) struct SchemaNode {
    #[serde(rename = "type", default)]
    kind: Option<SchemaType>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    pub(super) properties: Option<IndexMap<String, Self>>,
    #[serde(default)]
    required: Option<Vec<String>>,
    #[serde(rename = "enum", default)]
    enum_values: Option<Vec<Option<EnumValue>>>,
    #[serde(default)]
    items: Option<Box<Self>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SchemaType {
    Named(String),
    // JSON Schema allows a type array such as ["string", "null"], which LiteLLM renders as any
    Unrecognized(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EnumValue {
    Text(String),
    Number(Number),
    Bool(bool),
}

fn render_enum_value(value: Option<&EnumValue>) -> String {
    match value {
        Some(EnumValue::Text(text)) => format!("\"{text}\""),
        Some(EnumValue::Number(number)) => format!("\"{number}\""),
        Some(EnumValue::Bool(flag)) => format!("\"{flag}\""),
        None => "\"null\"".to_owned(),
    }
}

fn enum_rendering(node: &SchemaNode, otherwise: &str) -> String {
    let Some(values) = &node.enum_values else {
        return otherwise.to_owned();
    };
    let rendered: Vec<String> = values
        .iter()
        .map(Option::as_ref)
        .map(render_enum_value)
        .collect();
    rendered.join(" | ")
}

pub(super) fn object_parameters(
    node: &SchemaNode,
    indent: usize,
) -> Result<String, ToolFormatError> {
    let Some(properties) = &node.properties else {
        return Ok(String::new());
    };
    if properties.is_empty() {
        return Ok(String::new());
    }
    let required = node.required.as_deref().unwrap_or_default();
    let mut lines = Vec::new();
    for (key, property) in properties {
        if let Some(description) = &property.description {
            lines.push(format!("// {description}"));
        }
        let marker = if required.contains(key) { "" } else { "?" };
        let rendered = format_type(property, indent)?;
        lines.push(format!("{key}{marker}: {rendered},"));
    }
    let padding = " ".repeat(indent);
    Ok(lines
        .into_iter()
        .map(|line| format!("{padding}{line}"))
        .collect::<Vec<String>>()
        .join("\n"))
}

pub(super) fn format_type(node: &SchemaNode, indent: usize) -> Result<String, ToolFormatError> {
    let Some(SchemaType::Named(kind)) = &node.kind else {
        return Ok("any".to_owned());
    };
    match kind.as_str() {
        "string" => Ok(enum_rendering(node, "string")),
        "array" => {
            let Some(items) = node.items.as_deref() else {
                return Err(ToolFormatError::MissingItems { field: "items" });
            };
            let rendered = format_type(items, indent)?;
            Ok(format!("{rendered}[]"))
        }
        "object" => {
            let inner = object_parameters(node, indent.saturating_add(2))?;
            Ok(format!("{{\n{inner}\n}}"))
        }
        "integer" | "number" => Ok(enum_rendering(node, "number")),
        "boolean" => Ok("boolean".to_owned()),
        "null" => Ok("null".to_owned()),
        _ => Ok("any".to_owned()),
    }
}

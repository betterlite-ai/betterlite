use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

use crate::tools::error::ToolFormatError;

// a key LiteLLM reads only inside some type branch keeps any JSON value until that branch reads it
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Deferred<T> {
    Typed(T),
    Unread(IgnoredAny),
}

#[derive(Debug, Deserialize)]
pub(super) struct SchemaNode {
    #[serde(rename = "type", default)]
    kind: Option<SchemaType>,
    #[serde(default)]
    description: Option<Deferred<String>>,
    #[serde(default)]
    properties: Option<Deferred<IndexMap<String, Self>>>,
    #[serde(default)]
    required: Option<Deferred<Vec<String>>>,
    #[serde(rename = "enum", default)]
    enum_values: Option<Deferred<Vec<Option<EnumValue>>>>,
    #[serde(default)]
    items: Option<Deferred<Box<Self>>>,
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
    Unsupported(IgnoredAny),
}

impl SchemaNode {
    pub(super) const fn property_map(
        &self,
    ) -> Result<Option<&IndexMap<String, Self>>, ToolFormatError> {
        match &self.properties {
            Some(Deferred::Typed(properties)) => Ok(Some(properties)),
            Some(Deferred::Unread(_)) => Err(ToolFormatError::NotAnObject {
                field: "properties",
            }),
            None => Ok(None),
        }
    }
}

fn required_names(node: &SchemaNode) -> Result<&[String], ToolFormatError> {
    let Some(deferred) = &node.required else {
        return Ok(&[]);
    };
    match deferred {
        Deferred::Typed(names) => Ok(names),
        Deferred::Unread(_) => Err(ToolFormatError::NotTextArray { field: "required" }),
    }
}

fn render_enum_value(value: Option<&EnumValue>) -> Result<String, ToolFormatError> {
    match value {
        Some(EnumValue::Text(text)) => Ok(format!("\"{text}\"")),
        Some(EnumValue::Number(number)) => Ok(format!("\"{number}\"")),
        Some(EnumValue::Bool(flag)) => Ok(format!("\"{flag}\"")),
        Some(EnumValue::Unsupported(_)) => Err(ToolFormatError::NotScalar { field: "enum" }),
        None => Ok("\"null\"".to_owned()),
    }
}

fn enum_rendering(node: &SchemaNode, otherwise: &str) -> Result<String, ToolFormatError> {
    let Some(deferred) = &node.enum_values else {
        return Ok(otherwise.to_owned());
    };
    let Deferred::Typed(values) = deferred else {
        return Err(ToolFormatError::NotArray { field: "enum" });
    };
    let rendered = values
        .iter()
        .map(Option::as_ref)
        .map(render_enum_value)
        .collect::<Result<Vec<String>, ToolFormatError>>()?;
    Ok(rendered.join(" | "))
}

pub(super) fn object_parameters(
    node: &SchemaNode,
    indent: usize,
) -> Result<String, ToolFormatError> {
    let Some(properties) = node.property_map()? else {
        return Ok(String::new());
    };
    if properties.is_empty() {
        return Ok(String::new());
    }
    let required = required_names(node)?;
    let mut lines = Vec::new();
    for (key, property) in properties {
        match &property.description {
            Some(Deferred::Typed(text)) => lines.push(format!("// {text}")),
            Some(Deferred::Unread(_)) => {
                return Err(ToolFormatError::NotText {
                    field: "description",
                });
            }
            None => {}
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
        "string" => enum_rendering(node, "string"),
        "array" => {
            let Some(deferred) = &node.items else {
                return Err(ToolFormatError::MissingItems { field: "items" });
            };
            let Deferred::Typed(items) = deferred else {
                return Err(ToolFormatError::NotAnObject { field: "items" });
            };
            let rendered = format_type(items, indent)?;
            Ok(format!("{rendered}[]"))
        }
        "object" => {
            let inner = object_parameters(node, indent.saturating_add(2))?;
            Ok(format!("{{\n{inner}\n}}"))
        }
        "integer" | "number" => enum_rendering(node, "number"),
        "boolean" => Ok("boolean".to_owned()),
        "null" => Ok("null".to_owned()),
        _ => Ok("any".to_owned()),
    }
}

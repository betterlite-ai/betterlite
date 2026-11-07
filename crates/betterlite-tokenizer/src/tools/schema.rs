use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;
use serde_json::value::RawValue;

use crate::tools::error::ToolFormatError;

pub(super) const MAX_SCHEMA_DEPTH: usize = 64;

// every schema keyword stays a slice of the caller's text until the branch that reads it parses it
#[derive(Debug, Deserialize)]
pub(super) struct SchemaNode<'raw> {
    #[serde(rename = "type", default)]
    kind: Option<SchemaType>,
    #[serde(borrow, default)]
    description: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    properties: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    required: Option<&'raw RawValue>,
    #[serde(rename = "enum", borrow, default)]
    enum_values: Option<&'raw RawValue>,
    #[serde(borrow, default)]
    items: Option<&'raw RawValue>,
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

fn parse_field<'de, T>(raw: &'de RawValue, field: &'static str) -> Result<T, ToolFormatError>
where
    T: Deserialize<'de>,
{
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| ToolFormatError::Nested { field, source })
}

impl SchemaNode<'_> {
    fn properties(&self) -> Result<Option<IndexMap<String, Self>>, ToolFormatError> {
        let Some(raw) = self.properties else {
            return Ok(None);
        };
        parse_field(raw, "properties").map(Some)
    }

    fn required_names(&self) -> Result<Vec<String>, ToolFormatError> {
        let Some(raw) = self.required else {
            return Ok(Vec::new());
        };
        parse_field(raw, "required")
    }

    fn description_text(&self) -> Result<Option<String>, ToolFormatError> {
        let Some(raw) = self.description else {
            return Ok(None);
        };
        parse_field(raw, "description").map(Some)
    }

    fn enum_values(&self) -> Result<Option<Vec<Option<EnumValue>>>, ToolFormatError> {
        let Some(raw) = self.enum_values else {
            return Ok(None);
        };
        parse_field(raw, "enum").map(Some)
    }

    fn item_node(&self) -> Result<Option<Self>, ToolFormatError> {
        let Some(raw) = self.items else {
            return Ok(None);
        };
        parse_field(raw, "items").map(Some)
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

fn enum_rendering(node: &SchemaNode<'_>, otherwise: &str) -> Result<String, ToolFormatError> {
    let Some(values) = node.enum_values()? else {
        return Ok(otherwise.to_owned());
    };
    let rendered = values
        .iter()
        .map(Option::as_ref)
        .map(render_enum_value)
        .collect::<Result<Vec<String>, ToolFormatError>>()?;
    Ok(rendered.join(" | "))
}

pub(super) fn object_parameters(
    node: &SchemaNode<'_>,
    indent: usize,
    depth: usize,
) -> Result<String, ToolFormatError> {
    let Some(properties) = node.properties()? else {
        return Ok(String::new());
    };
    if properties.is_empty() {
        return Ok(String::new());
    }
    let required = node.required_names()?;
    let mut lines = Vec::new();
    for (key, property) in &properties {
        if let Some(text) = property.description_text()? {
            lines.push(format!("// {text}"));
        }
        let marker = if required.contains(key) { "" } else { "?" };
        let rendered = format_type(property, indent, depth.saturating_add(1))?;
        lines.push(format!("{key}{marker}: {rendered},"));
    }
    let padding = " ".repeat(indent);
    Ok(lines
        .into_iter()
        .map(|line| format!("{padding}{line}"))
        .collect::<Vec<String>>()
        .join("\n"))
}

pub(super) fn format_type(
    node: &SchemaNode<'_>,
    indent: usize,
    depth: usize,
) -> Result<String, ToolFormatError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(ToolFormatError::TooDeep {
            limit: MAX_SCHEMA_DEPTH,
        });
    }
    let Some(SchemaType::Named(kind)) = &node.kind else {
        return Ok("any".to_owned());
    };
    match kind.as_str() {
        "string" => enum_rendering(node, "string"),
        "array" => {
            let Some(items) = node.item_node()? else {
                return Err(ToolFormatError::MissingItems { field: "items" });
            };
            let rendered = format_type(&items, indent, depth.saturating_add(1))?;
            Ok(format!("{rendered}[]"))
        }
        "object" => {
            let inner = object_parameters(node, indent.saturating_add(2), depth)?;
            Ok(format!("{{\n{inner}\n}}"))
        }
        "integer" | "number" => enum_rendering(node, "number"),
        "boolean" => Ok("boolean".to_owned()),
        "null" => Ok("null".to_owned()),
        _ => Ok("any".to_owned()),
    }
}

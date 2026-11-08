use std::ops::Range;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

use crate::tools::error::ToolFormatError;
use crate::tools::json::{self, Field, Scanner};

pub(super) const MAX_SCHEMA_DEPTH: usize = 64;

#[derive(Debug, Default)]
pub(super) struct SchemaNode {
    kind: Option<Kind>,
    description: Option<Field<Range<usize>>>,
    properties: Option<Field<Members>>,
    required: Option<Field<Range<usize>>>,
    enum_values: Option<Field<Range<usize>>>,
    items: Option<Field<Box<Self>>>,
}

pub(super) type Members = IndexMap<String, Field<Box<SchemaNode>>>;

#[derive(Debug)]
enum Kind {
    Named(String),
    // JSON Schema allows a type array such as ["string", "null"], which LiteLLM renders as any
    Unrecognized,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EnumValue {
    Text(String),
    Number(Number),
    Bool(bool),
    Unsupported(IgnoredAny),
}

pub(super) fn parse_node(
    scanner: &mut Scanner<'_>,
    depth: usize,
) -> Result<SchemaNode, ToolFormatError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(ToolFormatError::TooDeep {
            limit: MAX_SCHEMA_DEPTH,
        });
    }
    let mut node = SchemaNode::default();
    scanner.take_byte(b'{')?;
    if scanner.peek() == Some(b'}') {
        scanner.take_byte(b'}')?;
        return Ok(node);
    }
    loop {
        if scanner.peek() != Some(b'"') {
            return Err(json::broken());
        }
        let key = scanner.parse_string("properties")?;
        scanner.take_byte(b':')?;
        assign(&mut node, scanner, &key, depth)?;
        match scanner.peek() {
            Some(b',') => scanner.take_byte(b',')?,
            Some(b'}') => {
                scanner.take_byte(b'}')?;
                return Ok(node);
            }
            Some(_) | None => return Err(json::broken()),
        }
    }
}

fn assign(
    node: &mut SchemaNode,
    scanner: &mut Scanner<'_>,
    key: &str,
    depth: usize,
) -> Result<(), ToolFormatError> {
    match key {
        "type" => node.kind = parse_kind(scanner)?,
        "description" => node.description = parse_leaf(scanner, b'"')?,
        "properties" => node.properties = parse_properties(scanner, depth)?,
        "required" => node.required = parse_leaf(scanner, b'[')?,
        "enum" => node.enum_values = parse_leaf(scanner, b'[')?,
        "items" => node.items = parse_item(scanner, depth)?,
        _ => {
            scanner.skip_value()?;
        }
    }
    Ok(())
}

fn parse_kind(scanner: &mut Scanner<'_>) -> Result<Option<Kind>, ToolFormatError> {
    match scanner.peek() {
        Some(b'"') => Ok(Some(Kind::Named(scanner.parse_string("type")?))),
        Some(b'n') => {
            scanner.skip_value()?;
            Ok(None)
        }
        Some(_) => {
            scanner.skip_value()?;
            Ok(Some(Kind::Unrecognized))
        }
        None => Err(json::broken()),
    }
}

fn parse_leaf(
    scanner: &mut Scanner<'_>,
    shape: u8,
) -> Result<Option<Field<Range<usize>>>, ToolFormatError> {
    match scanner.peek() {
        Some(byte) if byte == shape => {
            let range = scanner.skip_value()?;
            Ok(Some(Field::Value(range)))
        }
        Some(b'n') => {
            scanner.skip_value()?;
            Ok(None)
        }
        Some(byte) => {
            scanner.skip_value()?;
            Ok(Some(Field::Wrong(json::kind_name(byte))))
        }
        None => Err(json::broken()),
    }
}

fn parse_properties(
    scanner: &mut Scanner<'_>,
    depth: usize,
) -> Result<Option<Field<Members>>, ToolFormatError> {
    match scanner.peek() {
        Some(b'{') => Ok(Some(Field::Value(parse_members(scanner, depth)?))),
        Some(b'n') => {
            scanner.skip_value()?;
            Ok(None)
        }
        Some(byte) => {
            scanner.skip_value()?;
            Ok(Some(Field::Wrong(json::kind_name(byte))))
        }
        None => Err(json::broken()),
    }
}

fn parse_members(scanner: &mut Scanner<'_>, depth: usize) -> Result<Members, ToolFormatError> {
    let mut members = IndexMap::new();
    scanner.take_byte(b'{')?;
    if scanner.peek() == Some(b'}') {
        scanner.take_byte(b'}')?;
        return Ok(members);
    }
    loop {
        if scanner.peek() != Some(b'"') {
            return Err(json::broken());
        }
        let key = scanner.parse_string("properties")?;
        scanner.take_byte(b':')?;
        let child = parse_child(scanner, depth)?;
        members.insert(key, child);
        match scanner.peek() {
            Some(b',') => scanner.take_byte(b',')?,
            Some(b'}') => {
                scanner.take_byte(b'}')?;
                return Ok(members);
            }
            Some(_) | None => return Err(json::broken()),
        }
    }
}

fn parse_child(
    scanner: &mut Scanner<'_>,
    depth: usize,
) -> Result<Field<Box<SchemaNode>>, ToolFormatError> {
    match scanner.peek() {
        Some(b'{') => {
            let node = parse_node(scanner, depth.saturating_add(1))?;
            Ok(Field::Value(Box::new(node)))
        }
        Some(byte) => {
            scanner.skip_value()?;
            Ok(Field::Wrong(json::kind_name(byte)))
        }
        None => Err(json::broken()),
    }
}

fn parse_item(
    scanner: &mut Scanner<'_>,
    depth: usize,
) -> Result<Option<Field<Box<SchemaNode>>>, ToolFormatError> {
    match scanner.peek() {
        Some(b'{') => {
            let node = parse_node(scanner, depth.saturating_add(1))?;
            Ok(Some(Field::Value(Box::new(node))))
        }
        Some(b'n') => {
            scanner.skip_value()?;
            Ok(None)
        }
        Some(byte) => {
            scanner.skip_value()?;
            Ok(Some(Field::Wrong(json::kind_name(byte))))
        }
        None => Err(json::broken()),
    }
}

pub(super) fn object_parameters(
    source: &[u8],
    node: &SchemaNode,
    indent: usize,
    depth: usize,
) -> Result<String, ToolFormatError> {
    let Some(properties) = &node.properties else {
        return Ok(String::new());
    };
    let members = properties.value("properties", "a JSON object")?;
    if members.is_empty() {
        return Ok(String::new());
    }
    let required = required_names(source, node)?;
    let mut lines = Vec::new();
    for (key, field) in members {
        let property = field.value("properties", "a JSON object")?;
        if let Some(description) = &property.description {
            let range = description.value("description", "a string")?;
            let text = json::text(source, range, "description")?;
            lines.push(format!("// {text}"));
        }
        let marker = if required.contains(key) { "" } else { "?" };
        let rendered = format_type(source, property, indent, depth.saturating_add(1))?;
        lines.push(format!("{key}{marker}: {rendered},"));
    }
    let padding = " ".repeat(indent);
    Ok(lines
        .into_iter()
        .map(|line| format!("{padding}{line}"))
        .collect::<Vec<String>>()
        .join("\n"))
}

fn required_names(source: &[u8], node: &SchemaNode) -> Result<Vec<String>, ToolFormatError> {
    let Some(required) = &node.required else {
        return Ok(Vec::new());
    };
    let range = required.value("required", "an array of strings")?;
    let slice = source.get(range.clone()).ok_or_else(json::broken)?;
    serde_json::from_slice(slice).map_err(|failure| ToolFormatError::Nested {
        field: "required",
        source: failure,
    })
}

fn enum_rendering(
    source: &[u8],
    node: &SchemaNode,
    otherwise: &str,
) -> Result<String, ToolFormatError> {
    let Some(values) = &node.enum_values else {
        return Ok(otherwise.to_owned());
    };
    let range = values.value("enum", "an array")?;
    let slice = source.get(range.clone()).ok_or_else(json::broken)?;
    let entries: Vec<Option<EnumValue>> =
        serde_json::from_slice(slice).map_err(|failure| ToolFormatError::Nested {
            field: "enum",
            source: failure,
        })?;
    let rendered = entries
        .iter()
        .map(Option::as_ref)
        .map(render_enum_value)
        .collect::<Result<Vec<String>, ToolFormatError>>()?;
    Ok(rendered.join(" | "))
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

pub(super) fn format_type(
    source: &[u8],
    node: &SchemaNode,
    indent: usize,
    depth: usize,
) -> Result<String, ToolFormatError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(ToolFormatError::TooDeep {
            limit: MAX_SCHEMA_DEPTH,
        });
    }
    let Some(kind) = &node.kind else {
        return Ok("any".to_owned());
    };
    let Kind::Named(name) = kind else {
        return Ok("any".to_owned());
    };
    match name.as_str() {
        "string" => enum_rendering(source, node, "string"),
        "array" => {
            let Some(items) = &node.items else {
                return Err(ToolFormatError::MissingItems { field: "items" });
            };
            let item_node = items.value("items", "a JSON object")?;
            let rendered = format_type(source, item_node, indent, depth.saturating_add(1))?;
            Ok(format!("{rendered}[]"))
        }
        "object" => {
            let inner = object_parameters(source, node, indent.saturating_add(2), depth)?;
            Ok(format!("{{\n{inner}\n}}"))
        }
        "integer" | "number" => enum_rendering(source, node, "number"),
        "boolean" => Ok("boolean".to_owned()),
        "null" => Ok("null".to_owned()),
        _ => Ok("any".to_owned()),
    }
}

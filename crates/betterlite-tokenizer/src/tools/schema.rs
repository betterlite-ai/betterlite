use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;
use serde_json::value::RawValue;

use crate::tools::error::ToolFormatError;

pub(super) const MAX_SCHEMA_DEPTH: usize = 64;

pub(super) struct SchemaNode<'raw> {
    kind: Option<SchemaType>,
    description: Option<&'raw RawValue>,
    properties: Option<Properties<'raw>>,
    required: Option<&'raw RawValue>,
    enum_values: Option<&'raw RawValue>,
    items: Option<Item<'raw>>,
}

enum Properties<'raw> {
    Parsed(IndexMap<String, SchemaNode<'raw>>),
    Raw(&'raw RawValue),
}

enum Item<'raw> {
    Parsed(Box<SchemaNode<'raw>>),
    Raw(&'raw RawValue),
}

// the happy path: one pass over the schema, so the whole subtree is scanned exactly once
#[derive(Deserialize)]
#[serde(expecting = "a JSON schema object")]
struct StrictNode<'raw> {
    #[serde(rename = "type", default)]
    kind: Option<SchemaType>,
    #[serde(borrow, default)]
    description: Option<&'raw RawValue>,
    #[serde(default)]
    properties: Option<IndexMap<String, Self>>,
    #[serde(borrow, default)]
    required: Option<&'raw RawValue>,
    #[serde(rename = "enum", borrow, default)]
    enum_values: Option<&'raw RawValue>,
    #[serde(default)]
    items: Option<Box<Self>>,
}

// the fallback: recursive keywords stay raw so a wrong shape only fails where it is read
#[derive(Deserialize)]
#[serde(expecting = "a JSON schema object")]
struct LazyNode<'raw> {
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

impl<'raw> StrictNode<'raw> {
    fn into_schema(self) -> SchemaNode<'raw> {
        SchemaNode {
            kind: self.kind,
            description: self.description,
            properties: self.properties.map(|properties| {
                let mut parsed = IndexMap::with_capacity(properties.len());
                for (key, node) in properties {
                    parsed.insert(key, node.into_schema());
                }
                Properties::Parsed(parsed)
            }),
            required: self.required,
            enum_values: self.enum_values,
            items: self
                .items
                .map(|items| Item::Parsed(Box::new(items.into_schema()))),
        }
    }
}

impl<'raw> LazyNode<'raw> {
    fn into_schema(self) -> SchemaNode<'raw> {
        SchemaNode {
            kind: self.kind,
            description: self.description,
            properties: self.properties.map(Properties::Raw),
            required: self.required,
            enum_values: self.enum_values,
            items: self.items.map(Item::Raw),
        }
    }
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

pub(super) fn parse_schema<'raw>(
    raw: &'raw RawValue,
    field: &'static str,
) -> Result<SchemaNode<'raw>, ToolFormatError> {
    let mut strict = serde_json::Deserializer::from_str(raw.get());
    if let Ok(node) = serde_path_to_error::deserialize::<_, StrictNode<'raw>>(&mut strict) {
        return Ok(node.into_schema());
    }
    let mut lazy = serde_json::Deserializer::from_str(raw.get());
    serde_path_to_error::deserialize::<_, LazyNode<'raw>>(&mut lazy)
        .map(LazyNode::into_schema)
        .map_err(|source| ToolFormatError::Nested { field, source })
}

fn parse_children<'raw>(
    raw: &'raw RawValue,
    field: &'static str,
) -> Result<IndexMap<String, SchemaNode<'raw>>, ToolFormatError> {
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    let children: IndexMap<String, LazyNode<'_>> =
        serde_path_to_error::deserialize(&mut deserializer)
            .map_err(|source| ToolFormatError::Nested { field, source })?;
    Ok(children
        .into_iter()
        .map(|(key, node)| (key, node.into_schema()))
        .collect())
}

impl SchemaNode<'_> {
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
    let Some(properties) = &node.properties else {
        return Ok(String::new());
    };
    match properties {
        Properties::Parsed(parsed) => object_lines(node, parsed, indent, depth),
        Properties::Raw(raw) => {
            let parsed = parse_children(raw, "properties")?;
            object_lines(node, &parsed, indent, depth)
        }
    }
}

fn object_lines(
    node: &SchemaNode<'_>,
    properties: &IndexMap<String, SchemaNode<'_>>,
    indent: usize,
    depth: usize,
) -> Result<String, ToolFormatError> {
    if properties.is_empty() {
        return Ok(String::new());
    }
    let required = node.required_names()?;
    let mut lines = Vec::new();
    for (key, property) in properties {
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

fn format_item(
    node: &SchemaNode<'_>,
    indent: usize,
    depth: usize,
) -> Result<String, ToolFormatError> {
    let Some(item) = &node.items else {
        return Err(ToolFormatError::MissingItems { field: "items" });
    };
    let next = depth.saturating_add(1);
    match item {
        Item::Parsed(items) => format_type(items, indent, next),
        Item::Raw(raw) => {
            let items = parse_schema(raw, "items")?;
            format_type(&items, indent, next)
        }
    }
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
            let rendered = format_item(node, indent, depth)?;
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

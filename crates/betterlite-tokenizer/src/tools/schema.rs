use indexmap::IndexMap;
use serde_json::value::RawValue;

use crate::tools::error::ToolFormatError;

pub(super) type Fields = IndexMap<String, Box<RawValue>>;

// LiteLLM reads a missing key and an explicit null the same way
pub(super) fn present<'raw>(fields: &'raw Fields, key: &str) -> Option<&'raw RawValue> {
    let raw = fields.get(key)?;
    if raw.get() == "null" {
        return None;
    }
    Some(raw.as_ref())
}

pub(super) fn text(fields: &Fields, key: &'static str) -> Result<Option<String>, ToolFormatError> {
    let Some(raw) = present(fields, key) else {
        return Ok(None);
    };
    serde_json::from_str::<String>(raw.get())
        .map_or(Err(ToolFormatError::NotText { field: key }), |value| {
            Ok(Some(value))
        })
}

pub(super) fn object_from(
    value: Option<&RawValue>,
    key: &'static str,
) -> Result<Option<Fields>, ToolFormatError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    match serde_json::from_str(raw.get()) {
        Ok(fields) => Ok(Some(fields)),
        Err(_error) => Err(ToolFormatError::NotAnObject { field: key }),
    }
}

fn required_names(fields: &Fields) -> Result<Vec<String>, ToolFormatError> {
    let Some(raw) = present(fields, "required") else {
        return Ok(Vec::new());
    };
    serde_json::from_str(raw.get())
        .map_err(|_error| ToolFormatError::NotTextArray { field: "required" })
}

fn enum_value(raw: &RawValue) -> Result<String, ToolFormatError> {
    if let Ok(text) = serde_json::from_str::<String>(raw.get()) {
        return Ok(format!("\"{text}\""));
    }
    let text = raw.get();
    if text.starts_with('{') || text.starts_with('[') {
        return Err(ToolFormatError::NotScalar { field: "enum" });
    }
    Ok(format!("\"{text}\""))
}

fn enum_rendering(fields: &Fields, otherwise: &str) -> Result<String, ToolFormatError> {
    let Some(raw) = present(fields, "enum") else {
        return Ok(otherwise.to_owned());
    };
    let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())
        .map_err(|_error| ToolFormatError::NotAnArray { field: "enum" })?;
    let rendered = values
        .iter()
        .map(|value| enum_value(value))
        .collect::<Result<Vec<String>, ToolFormatError>>()?;
    Ok(rendered.join(" | "))
}

pub(super) fn object_parameters(fields: &Fields, indent: usize) -> Result<String, ToolFormatError> {
    let Some(properties) = object_from(present(fields, "properties"), "properties")? else {
        return Ok(String::new());
    };
    if properties.is_empty() {
        return Ok(String::new());
    }
    let required = required_names(fields)?;
    let mut lines = Vec::new();
    for (key, property_raw) in &properties {
        let property = serde_json::from_str::<Fields>(property_raw.get())
            .ok()
            .ok_or(ToolFormatError::NotAnObject {
                field: "properties",
            })?;
        if let Some(description) = text(&property, "description")? {
            lines.push(format!("// {description}"));
        }
        let marker = if required.contains(key) { "" } else { "?" };
        let rendered = format_type(&property, indent)?;
        lines.push(format!("{key}{marker}: {rendered},"));
    }
    let padding = " ".repeat(indent);
    Ok(lines
        .into_iter()
        .map(|line| format!("{padding}{line}"))
        .collect::<Vec<String>>()
        .join("\n"))
}

pub(super) fn format_type(fields: &Fields, indent: usize) -> Result<String, ToolFormatError> {
    let kind = fields
        .get("type")
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok());
    match kind.as_deref() {
        Some("string") => enum_rendering(fields, "string"),
        Some("array") => {
            let raw =
                present(fields, "items").ok_or(ToolFormatError::MissingItems { field: "items" })?;
            let items = serde_json::from_str::<Fields>(raw.get())
                .ok()
                .ok_or(ToolFormatError::NotAnObject { field: "items" })?;
            let rendered = format_type(&items, indent)?;
            Ok(format!("{rendered}[]"))
        }
        Some("object") => {
            let inner = object_parameters(fields, indent.saturating_add(2))?;
            Ok(format!("{{\n{inner}\n}}"))
        }
        Some("integer" | "number") => enum_rendering(fields, "number"),
        Some("boolean") => Ok("boolean".to_owned()),
        Some("null") => Ok("null".to_owned()),
        _ => Ok("any".to_owned()),
    }
}

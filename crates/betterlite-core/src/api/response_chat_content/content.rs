use serde_json::{Map, Value};

use super::debug::{display_text_value, format_diagnostic_value, text_block};
use super::image_file::{image_and_file_fields, input_file};
use super::{ContentError, Projection};

type ItemProjection = (Value, String, Option<String>);
type ItemResult = Option<Result<ItemProjection, ContentError>>;

#[must_use]
pub fn text_by_role(content: &Value, role: &str) -> Value {
    match content {
        Value::Null => Value::Array(vec![text_block("", role).0]),
        Value::String(text) => Value::Array(vec![text_block(text, role).0]),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.as_str() {
                        return Some(text_block(text, role).0);
                    }
                    let fields = item.as_object()?;
                    let text = if fields.get("type").and_then(Value::as_str) == Some("text") {
                        fields.get("text").and_then(Value::as_str).unwrap_or("")
                    } else {
                        fields.get("text")?.as_str()?
                    };
                    Some(text_block(text, role).0)
                })
                .collect(),
        ),
        Value::Object(fields) => Value::Array(vec![
            text_block(
                &display_text_value(fields.get("text").unwrap_or(content)),
                role,
            )
            .0,
        ]),
        Value::Bool(_) | Value::Number(_) => {
            Value::Array(vec![text_block(&display_text_value(content), role).0])
        }
    }
}

pub fn content_passthrough(items: &[Value], role: &str) -> Result<Projection, ContentError> {
    let mut result = Vec::new();
    let mut result_reprs = Vec::new();
    let mut diagnostics = Vec::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("tool_reference") {
            diagnostics.push(
                "Chat provider:   tool_reference has no responses API equivalent; skipped"
                    .to_owned(),
            );
            continue;
        }
        let Some((value, repr, diagnostic)) = project_item(item, role).transpose()? else {
            continue;
        };
        diagnostics.extend(diagnostic);
        result.push(value);
        result_reprs.push(repr);
    }
    diagnostics.push(format!(
        "Chat provider: Final converted content: [{}]",
        result_reprs.join(", ")
    ));
    Ok(Projection {
        value: Value::Array(result),
        diagnostics,
    })
}

fn project_item(item: &Value, role: &str) -> ItemResult {
    item.as_str().map_or_else(
        || {
            item.as_object()
                .map(|fields| project_object(item, fields, role))
        },
        |text| {
            let (value, repr) = text_block(text, role);
            Some(Ok((
                value,
                repr.clone(),
                Some(format!("Chat provider:   -> {repr}")),
            )))
        },
    )
}

fn project_object(
    item: &Value,
    fields: &Map<String, Value>,
    role: &str,
) -> Result<ItemProjection, ContentError> {
    let Some(kind) = fields.get("type").and_then(Value::as_str) else {
        return Ok(diagnostic_result(item.clone(), "passthrough"));
    };
    match kind {
        "text" => {
            let text = fields.get("text").and_then(Value::as_str).unwrap_or("");
            let (value, repr) = text_block(text, role);
            Ok((
                value,
                repr.clone(),
                Some(format!("Chat provider:   text -> {repr}")),
            ))
        }
        "image" => Ok(image_item(fields)),
        "image_url" => image_and_file_fields(fields).map(image_url_result),
        "file" => Ok(diagnostic_result(input_file(fields.get("file")), "file")),
        "input_text"
        | "input_image"
        | "output_text"
        | "refusal"
        | "input_file"
        | "computer_screenshot"
        | "summary_text"
        | "tool_reference" => Ok(diagnostic_result(item.clone(), "passthrough")),
        _ => Ok(unknown_item(item, fields, kind, role)),
    }
}

fn image_url_result(value: Value) -> ItemProjection {
    let image_url = value
        .get("image_url")
        .map_or_else(|| "None".to_owned(), format_diagnostic_value);
    let detail = value
        .get("detail")
        .map_or_else(|| "None".to_owned(), format_diagnostic_value);
    let kind = value
        .get("type")
        .map_or_else(|| "None".to_owned(), format_diagnostic_value);
    let repr = format!("{{'image_url': {image_url}, 'detail': {detail}, 'type': {kind}}}");
    (
        value,
        repr.clone(),
        Some(format!("Chat provider:   image_url -> {repr}")),
    )
}

fn diagnostic_result(value: Value, label: &str) -> ItemProjection {
    let repr = format_diagnostic_value(&value);
    (
        value,
        repr.clone(),
        Some(format!("Chat provider:   {label} -> {repr}")),
    )
}

fn image_item(fields: &Map<String, Value>) -> ItemProjection {
    let mut image = Map::new();
    image.insert("type".to_owned(), Value::String("input_image".to_owned()));
    image.extend(fields.clone());
    let value = Value::Object(image);
    let repr = format_diagnostic_value(&value);
    (
        value,
        repr.clone(),
        Some(format!("Chat provider:   image -> {repr}")),
    )
}

fn unknown_item(
    item: &Value,
    fields: &Map<String, Value>,
    kind: &str,
    role: &str,
) -> ItemProjection {
    let value = item.clone();
    let text = fields
        .get("text")
        .map_or_else(|| display_text_value(&value), display_text_value);
    let (converted, repr) = text_block(&text, role);
    (
        converted,
        repr.clone(),
        Some(format!("Chat provider:   unknown({kind}) -> {repr}")),
    )
}

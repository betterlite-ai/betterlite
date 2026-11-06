use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::agents::json::AgentJson;

use crate::client::convert::ConvertError;
use crate::client::convert::{named, push_optional, text_field};

pub fn to_core_part(part: &AgentJson) -> Result<AgentJson, ConvertError> {
    match part.get("kind").and_then(AgentJson::as_str) {
        Some("text") => Ok(text_to_core(part)),
        Some("data") => data_to_core(part),
        Some("file") => file_to_core(part),
        Some(unknown) => Err(ConvertError::UnknownPart {
            kind: unknown.to_owned(),
        }),
        None => Err(ConvertError::UnknownPart {
            kind: String::new(),
        }),
    }
}

fn text_to_core(part: &AgentJson) -> AgentJson {
    let mut fields = named(vec![(
        "text",
        part.get("text").cloned().unwrap_or_default(),
    )]);
    push_optional(&mut fields, "metadata", part.get("metadata").cloned());
    AgentJson::Object(fields.into_iter().collect())
}

fn data_to_core(part: &AgentJson) -> Result<AgentJson, ConvertError> {
    let metadata = part.get("metadata");
    let compat = metadata
        .and_then(|value| value.get("data_part_compat"))
        .and_then(AgentJson::as_bool)
        .unwrap_or(false);
    let raw = part.get("data").cloned().unwrap_or_default();
    let data = if compat {
        raw.get("value")
            .cloned()
            .ok_or(ConvertError::MissingDataValue)?
    } else {
        raw
    };
    let mut fields = named(vec![("data", data)]);
    push_optional(&mut fields, "metadata", stripped_metadata(metadata));
    Ok(AgentJson::Object(fields.into_iter().collect()))
}
fn stripped_metadata(metadata: Option<&AgentJson>) -> Option<AgentJson> {
    let mut fields = metadata?.as_object()?.clone();
    fields.remove("data_part_compat");
    (!fields.is_empty()).then_some(AgentJson::Object(fields))
}

fn file_to_core(part: &AgentJson) -> Result<AgentJson, ConvertError> {
    let file = part.get("file").cloned().unwrap_or_default();
    let mut fields = named(vec![file_content(&file)?]);
    copy_file_fields(&file, &mut fields, part);
    Ok(AgentJson::Object(fields.into_iter().collect()))
}
fn file_content(file: &AgentJson) -> Result<(&'static str, AgentJson), ConvertError> {
    if let Some(bytes) = file.get("bytes").and_then(AgentJson::as_str) {
        let decoded = STANDARD.decode(bytes).map_err(ConvertError::FileBytes)?;
        return Ok(("raw", AgentJson::String(STANDARD.encode(decoded))));
    }
    let Some(uri) = file.get("uri").and_then(AgentJson::as_str) else {
        return Err(ConvertError::FileContent);
    };
    Ok(("url", AgentJson::String(uri.to_owned())))
}
fn copy_file_fields(file: &AgentJson, fields: &mut Vec<(String, AgentJson)>, part: &AgentJson) {
    push_optional(fields, "mediaType", text_field(file, "mimeType"));
    push_optional(fields, "filename", text_field(file, "name"));
    push_optional(fields, "metadata", part.get("metadata").cloned());
}

pub fn parts_to_compat(parts: Option<&AgentJson>) -> Result<AgentJson, ConvertError> {
    let Some(AgentJson::Array(items)) = parts else {
        return Ok(AgentJson::Array(Vec::new()));
    };
    let converted = items
        .iter()
        .map(to_compat_part)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AgentJson::Array(converted))
}

pub fn to_compat_part(part: &AgentJson) -> Result<AgentJson, ConvertError> {
    if let Some(text) = part.get("text") {
        let mut fields = named(vec![
            ("kind", AgentJson::String("text".to_owned())),
            ("text", text.clone()),
        ]);
        push_optional(&mut fields, "metadata", part.get("metadata").cloned());
        return Ok(AgentJson::Object(fields.into_iter().collect()));
    }
    if let Some(data) = part.get("data") {
        return Ok(data_to_compat(data, part.get("metadata")));
    }
    if let Some(raw) = part.get("raw") {
        return Ok(file_to_compat("bytes", raw.clone(), part));
    }
    if let Some(url) = part.get("url") {
        return Ok(file_to_compat("uri", url.clone(), part));
    }
    Err(ConvertError::UnknownContent)
}

fn data_to_compat(data: &AgentJson, metadata: Option<&AgentJson>) -> AgentJson {
    let mut fields = named(vec![("kind", AgentJson::String("data".to_owned()))]);
    if data.is_object() {
        fields.push(("data".to_owned(), data.clone()));
        push_optional(&mut fields, "metadata", metadata.cloned());
    } else {
        let wrapped = AgentJson::Object(named(vec![("value", data.clone())]).into_iter().collect());
        fields.push(("data".to_owned(), wrapped));
        fields.push(("metadata".to_owned(), flagged_metadata(metadata)));
    }
    AgentJson::Object(fields.into_iter().collect())
}

fn flagged_metadata(existing: Option<&AgentJson>) -> AgentJson {
    let mut metadata = existing.cloned().unwrap_or_default();
    match &mut metadata {
        AgentJson::Object(fields) => {
            fields.insert("data_part_compat".to_owned(), AgentJson::Bool(true));
        }
        other @ (AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::String(_)
        | AgentJson::Array(_)) => {
            *other = AgentJson::Object(
                named(vec![("data_part_compat", AgentJson::Bool(true))])
                    .into_iter()
                    .collect(),
            );
        }
    }
    metadata
}

fn file_to_compat(key: &str, content: AgentJson, part: &AgentJson) -> AgentJson {
    let mut file = named(vec![(key, content)]);
    push_optional(&mut file, "mimeType", text_field(part, "mediaType"));
    push_optional(&mut file, "name", text_field(part, "filename"));
    let mut fields = named(vec![
        ("kind", AgentJson::String("file".to_owned())),
        ("file", AgentJson::Object(file.into_iter().collect())),
    ]);
    push_optional(&mut fields, "metadata", part.get("metadata").cloned());
    AgentJson::Object(fields.into_iter().collect())
}

use std::iter::once;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::agents::json::AgentJson;

use crate::client::convert::ConvertError;
use crate::client::records::compat::{
    DataPart, FileContent, FilePart, FileWithBytes, FileWithUri, Part, TextPart,
};
use crate::client::records::core::{
    CoreDataPart, CorePart, CoreRawPart, CoreTextPart, CoreUrlPart,
};

pub fn to_core_part(part: Part) -> Result<CorePart, ConvertError> {
    match part {
        Part::Text(text) => Ok(CorePart::Text(CoreTextPart {
            text: text.text,
            metadata: text.metadata,
        })),
        Part::Data(data) => data_to_core(data),
        Part::File(file) => file_to_core(file),
    }
}

fn data_to_core(part: DataPart) -> Result<CorePart, ConvertError> {
    let data = core_data(&part.data, part.metadata.as_ref())?;
    Ok(CorePart::Data(CoreDataPart {
        data,
        metadata: stripped_metadata(part.metadata),
    }))
}

fn core_data(data: &AgentJson, metadata: Option<&AgentJson>) -> Result<AgentJson, ConvertError> {
    let compat = metadata
        .and_then(|value| value.get("data_part_compat"))
        .and_then(AgentJson::as_bool)
        .unwrap_or(false);
    if !compat {
        return Ok(data.clone());
    }
    data.get("value")
        .cloned()
        .ok_or(ConvertError::MissingDataValue)
}

fn stripped_metadata(metadata: Option<AgentJson>) -> Option<AgentJson> {
    let mut fields = metadata?.as_object()?.clone();
    fields.remove("data_part_compat");
    (!fields.is_empty()).then_some(AgentJson::Object(fields))
}

fn file_to_core(part: FilePart) -> Result<CorePart, ConvertError> {
    match part.file {
        FileContent::Bytes(file) => {
            // base64.b64decode without validation ignores every character outside the alphabet, such as line wraps
            let encoded: Vec<u8> = file
                .bytes
                .bytes()
                .filter(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
                .collect();
            let decoded = STANDARD.decode(&encoded).map_err(ConvertError::FileBytes)?;
            Ok(CorePart::Raw(CoreRawPart {
                raw: STANDARD.encode(decoded),
                media_type: file.mime_type.filter(|value| !value.is_empty()),
                filename: file.name.filter(|value| !value.is_empty()),
                metadata: part.metadata,
            }))
        }
        FileContent::Uri(file) => Ok(CorePart::Url(CoreUrlPart {
            url: file.uri,
            media_type: file.mime_type.filter(|value| !value.is_empty()),
            filename: file.name.filter(|value| !value.is_empty()),
            metadata: part.metadata,
        })),
    }
}

#[must_use]
pub fn to_compat_part(part: &CorePart) -> Part {
    match part {
        CorePart::Text(text) => Part::Text(TextPart {
            text: text.text.clone(),
            metadata: text.metadata.clone(),
        }),
        CorePart::Data(data) => Part::Data(data_to_compat(data)),
        CorePart::Raw(raw) => Part::File(FilePart {
            file: FileContent::Bytes(FileWithBytes {
                bytes: raw.raw.clone(),
                mime_type: raw.media_type.clone().filter(|value| !value.is_empty()),
                name: raw.filename.clone().filter(|value| !value.is_empty()),
            }),
            metadata: raw.metadata.clone(),
        }),
        CorePart::Url(url) => Part::File(FilePart {
            file: FileContent::Uri(FileWithUri {
                uri: url.url.clone(),
                mime_type: url.media_type.clone().filter(|value| !value.is_empty()),
                name: url.filename.clone().filter(|value| !value.is_empty()),
            }),
            metadata: url.metadata.clone(),
        }),
    }
}

fn data_to_compat(part: &CoreDataPart) -> DataPart {
    if part.data.is_object() {
        return DataPart {
            data: part.data.clone(),
            metadata: part.metadata.clone(),
        };
    }
    DataPart {
        data: AgentJson::Object(once(("value".to_owned(), part.data.clone())).collect()),
        metadata: Some(flagged_metadata(part.metadata.as_ref())),
    }
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
                once(("data_part_compat".to_owned(), AgentJson::Bool(true))).collect(),
            );
        }
    }
    metadata
}

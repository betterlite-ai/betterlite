use std::io::{Cursor, Read as _, Seek as _, SeekFrom};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::openai_video_identifiers::multipart::{MediaBody, MultipartPart};
use betterlite_openai::video_edits::character::{CharacterPart, CreateCharacter};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::helper_error;
use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Fixture {
    Bytes {
        data_base64: String,
    },
    Text {
        value: String,
    },
    Stream {
        data_base64: String,
        #[serde(default)]
        name: Option<String>,
    },
}

enum PreparedKind {
    Bytes(Vec<u8>),
    Text(String),
    Reader(Cursor<Vec<u8>>),
}

pub(super) struct Prepared {
    name: Option<String>,
    kind: PreparedKind,
}

impl Prepared {
    pub(super) fn media(&mut self) -> MediaBody<'_> {
        match &mut self.kind {
            PreparedKind::Bytes(bytes) => MediaBody::Bytes(bytes.as_slice()),
            PreparedKind::Text(text) => MediaBody::Text(text.as_str()),
            PreparedKind::Reader(reader) => MediaBody::Stream {
                name: self.name.as_deref(),
                reader,
            },
        }
    }
}

pub(super) fn prepared(fixture: &Fixture) -> Result<Prepared, Box<HelperError>> {
    match fixture {
        Fixture::Bytes { data_base64 } => Ok(Prepared {
            name: None,
            kind: PreparedKind::Bytes(decode(data_base64)?),
        }),
        Fixture::Text { value } => Ok(Prepared {
            name: None,
            kind: PreparedKind::Text(value.clone()),
        }),
        Fixture::Stream { data_base64, name } => Ok(Prepared {
            name: name.clone(),
            kind: PreparedKind::Reader(Cursor::new(decode(data_base64)?)),
        }),
    }
}

fn decode(data_base64: &str) -> Result<Vec<u8>, Box<HelperError>> {
    STANDARD.decode(data_base64).map_err(|error| {
        helper_error(
            "InvalidInput",
            format!("the fixture data_base64 is not base64: {error}"),
        )
    })
}

pub(super) fn part_value(
    field: &str,
    filename: Option<&str>,
    content_type: Option<&str>,
    kind: &str,
    body: &Value,
) -> Value {
    json!({
        "field": field,
        "filename": filename,
        "content_type": content_type,
        "body_kind": kind,
        "body": body,
    })
}

pub(super) fn file_part_value(file: &mut MultipartPart<'_>) -> Result<Value, Box<HelperError>> {
    let (kind, body) = match &mut file.body {
        MediaBody::Bytes(bytes) => ("bytes", STANDARD.encode(bytes)),
        MediaBody::Text(text) => ("text", (*text).to_owned()),
        MediaBody::Stream { reader, .. } => {
            let mut data = Vec::new();
            reader
                .seek(SeekFrom::Start(0))
                .and_then(|_| reader.read_to_end(&mut data))
                .map_err(|error| helper_error("IoError", error.to_string()))?;
            ("stream", STANDARD.encode(data))
        }
    };
    Ok(part_value(
        file.field.as_str(),
        Some(file.filename.as_str()),
        Some(file.content_type),
        kind,
        &Value::String(body),
    ))
}

pub(super) fn character_files(
    created: &mut CreateCharacter<'_>,
) -> Result<Value, Box<HelperError>> {
    let parts = created
        .files
        .iter_mut()
        .map(|part| match part {
            CharacterPart::Text { field, text } => {
                Ok(part_value(field, None, None, "text", &json!(text)))
            }
            CharacterPart::File(file) => file_part_value(file),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(parts))
}

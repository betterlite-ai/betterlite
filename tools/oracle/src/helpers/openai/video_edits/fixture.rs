use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::video_edits::character::{CharacterPart, CreateCharacter};
use serde::Deserialize;
use serde_json::{Value, json};

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

    pub(super) fn body(&self) -> Value {
        match &self.kind {
            PreparedKind::Bytes(bytes) => Value::String(STANDARD.encode(bytes)),
            PreparedKind::Text(text) => Value::String(text.clone()),
            PreparedKind::Reader(_) => Value::Null,
        }
    }

    pub(super) fn body_kind(&self) -> &'static str {
        match &self.kind {
            PreparedKind::Bytes(_) => "bytes",
            PreparedKind::Text(_) => "text",
            PreparedKind::Reader(_) => "stream",
        }
    }
}

pub(super) fn prepared(fixture: &Fixture) -> Prepared {
    match fixture {
        Fixture::Bytes { data_base64 } => Prepared {
            name: None,
            kind: PreparedKind::Bytes(STANDARD.decode(data_base64).unwrap_or_default()),
        },
        Fixture::Text { value } => Prepared {
            name: None,
            kind: PreparedKind::Text(value.clone()),
        },
        Fixture::Stream { data_base64, name } => Prepared {
            name: name.clone(),
            kind: PreparedKind::Reader(Cursor::new(
                STANDARD.decode(data_base64).unwrap_or_default(),
            )),
        },
    }
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

pub(super) fn character_files(created: &CreateCharacter<'_>, kind: &str, body: &Value) -> Value {
    Value::Array(
        created
            .files
            .iter()
            .map(|part| match part {
                CharacterPart::Text { field, text } => {
                    part_value(*field, None, None, "text", &json!(*text))
                }
                CharacterPart::File(file) => part_value(
                    file.field.as_str(),
                    Some(file.filename.as_str()),
                    Some(file.content_type),
                    kind,
                    body,
                ),
            })
            .collect(),
    )
}

use std::io::{self, Read, Seek, SeekFrom};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Fixture {
    Bytes {
        data_base64: String,
    },
    Text {
        value: String,
    },
    BytesIo {
        data_base64: String,
        position: usize,
    },
    BufferedReader {
        data_base64: String,
        position: usize,
        name: String,
    },
    FailingReader {
        data_base64: String,
        position: usize,
        failure: Failure,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Failure {
    #[serde(rename = "none")]
    None,
    Tell,
    Seek,
    Read,
    Restore,
}

pub(super) enum PreparedKind {
    Bytes(Vec<u8>),
    Text(String),
    Reader(ProbeReader),
}

pub(super) struct Prepared {
    name: Option<String>,
    kind: PreparedKind,
}

impl Prepared {
    pub(super) fn media_body(&mut self) -> MediaBody<'_> {
        let name = self.name.as_deref();
        match &mut self.kind {
            PreparedKind::Bytes(bytes) => MediaBody::Bytes(bytes.as_slice()),
            PreparedKind::Text(text) => MediaBody::Text(text.as_str()),
            PreparedKind::Reader(reader) => MediaBody::Stream { name, reader },
        }
    }

    pub(super) fn position(&self) -> Value {
        match &self.kind {
            PreparedKind::Reader(reader) => json!(reader.position),
            PreparedKind::Bytes(_) | PreparedKind::Text(_) => Value::Null,
        }
    }
}

pub(super) struct ProbeReader {
    data: Vec<u8>,
    position: usize,
    failure: Failure,
    seeks: usize,
}

impl ProbeReader {
    fn new(data: Vec<u8>, position: usize, failure: Failure) -> Self {
        Self {
            data,
            position,
            failure,
            seeks: 0,
        }
    }
}

impl Read for ProbeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if matches!(self.failure, Failure::Read) {
            return Err(io::Error::other("read failed"));
        }
        let remaining = self.data.len().saturating_sub(self.position);
        let taken = remaining.min(buffer.len());
        let source = self
            .data
            .get(self.position..self.position.saturating_add(taken))
            .unwrap_or_default();
        buffer
            .get_mut(..taken)
            .unwrap_or_default()
            .copy_from_slice(source);
        self.position = self.position.saturating_add(taken);
        Ok(taken)
    }
}

impl Seek for ProbeReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        self.seeks = self.seeks.saturating_add(1);
        if matches!(self.failure, Failure::Seek) && self.seeks == 1 {
            return Err(io::Error::other("seek failed"));
        }
        if matches!(self.failure, Failure::Restore) && self.seeks > 1 {
            return Err(io::Error::other("restore failed"));
        }
        if let SeekFrom::Start(offset) = target {
            self.position =
                usize::try_from(offset).map_err(|error| io::Error::other(error.to_string()))?;
        }
        Ok(u64::try_from(self.position).unwrap_or_default())
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        if matches!(self.failure, Failure::Tell) {
            return Err(io::Error::other("tell failed"));
        }
        Ok(u64::try_from(self.position).unwrap_or_default())
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
        Fixture::BytesIo {
            data_base64,
            position,
        } => Prepared {
            name: None,
            kind: PreparedKind::Reader(ProbeReader::new(
                STANDARD.decode(data_base64).unwrap_or_default(),
                *position,
                Failure::None,
            )),
        },
        Fixture::BufferedReader {
            data_base64,
            position,
            name,
        } => Prepared {
            name: Some(name.clone()),
            kind: PreparedKind::Reader(ProbeReader::new(
                STANDARD.decode(data_base64).unwrap_or_default(),
                *position,
                Failure::None,
            )),
        },
        Fixture::FailingReader {
            data_base64,
            position,
            failure,
        } => Prepared {
            name: None,
            kind: PreparedKind::Reader(ProbeReader::new(
                STANDARD.decode(data_base64).unwrap_or_default(),
                *position,
                *failure,
            )),
        },
    }
}

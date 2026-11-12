use std::collections::BTreeMap;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioStream, AudioTuple, AuthorizedAudioFiles, TupleContent,
};
use serde::Deserialize;
use serde::de::IgnoredAny;

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum File {
    Tuple(TupleFile),
    Part(Part),
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum TupleKind {
    Tuple,
}

#[derive(Clone, Deserialize)]
pub(super) struct TupleFile {
    #[serde(rename = "kind")]
    _kind: TupleKind,
    filename: Option<String>,
    #[serde(default)]
    content: Option<Part>,
    #[serde(rename = "mime")]
    _mime: Option<IgnoredAny>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct Base64(Vec<u8>);

impl TryFrom<String> for Base64 {
    type Error = base64::DecodeError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        BASE64.decode(text).map(Self)
    }
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(super) enum Part {
    Bytes { content: Base64 },
    ByteArray { content: Base64 },
    Text { content: String },
    Path { path: PathBuf },
    Stream(FixtureStream),
    Unsupported { class_name: String },
}

#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
enum ReadMode {
    #[default]
    Content,
    Fail,
    Missing,
}

#[derive(Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SeekMode {
    #[default]
    Works,
    Unsupported,
    Fails,
}

#[derive(Clone, Deserialize)]
pub(super) struct FixtureStream {
    name: String,
    #[serde(default)]
    content: Base64,
    #[serde(default)]
    position: usize,
    #[serde(default)]
    read: ReadMode,
    #[serde(default)]
    seek: SeekMode,
}

impl FixtureStream {
    fn move_to(&mut self, position: u64) -> io::Result<()> {
        if self.seek == SeekMode::Fails {
            return Err(io::Error::other("the fixture seek fails"));
        }
        self.position = usize::try_from(position).map_err(|_| io::Error::other("position"))?;
        Ok(())
    }
}

impl AudioStream for FixtureStream {
    fn name(&self) -> Option<&str> {
        Some(self.name.as_str())
    }

    fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.read {
            ReadMode::Fail => Err(io::Error::other("the fixture reader fails")),
            ReadMode::Missing => Ok(None),
            ReadMode::Content => {
                let start = self.position.min(self.content.0.len());
                let content = self.content.0.get(start..).map(<[u8]>::to_vec);
                self.position = self.content.0.len();
                Ok(content)
            }
        }
    }

    fn tell(&mut self) -> Option<io::Result<u64>> {
        Some(u64::try_from(self.position).map_err(|_| io::Error::other("position")))
    }

    fn seek(&mut self, position: u64) -> Option<io::Result<()>> {
        match self.seek {
            SeekMode::Unsupported => None,
            SeekMode::Works | SeekMode::Fails => Some(self.move_to(position)),
        }
    }
}

pub(super) struct FixtureFiles(pub(super) BTreeMap<PathBuf, Vec<u8>>);

impl AuthorizedAudioFiles for FixtureFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(Cursor::new(self.read(path)?)))
    }
}

// AudioFile and TupleContent list the same variants, so one match serves both
macro_rules! borrow_part {
    ($part:expr, $target:ident) => {
        match $part {
            Part::Bytes { content } => $target::Bytes(&content.0),
            Part::ByteArray { content } => $target::ByteArray(&content.0),
            Part::Text { content } => $target::Text(content),
            Part::Path { path } => $target::Path(path),
            Part::Stream(stream) => $target::Stream(stream),
            Part::Unsupported { class_name } => $target::Unsupported {
                python_class: class_name,
            },
        }
    };
}

impl File {
    pub(super) fn as_file(&mut self) -> AudioFile<'_> {
        match self {
            Self::Part(part) => borrow_part!(part, AudioFile),
            Self::Tuple(tuple) => AudioFile::Tuple(AudioTuple {
                filename: tuple.filename.as_deref().unwrap_or_default(),
                content: tuple
                    .content
                    .as_mut()
                    .map(|part| borrow_part!(part, TupleContent)),
            }),
        }
    }

    pub(super) const fn stream_position(&self) -> Option<usize> {
        match self {
            Self::Part(Part::Stream(stream))
            | Self::Tuple(TupleFile {
                content: Some(Part::Stream(stream)),
                ..
            }) => Some(stream.position),
            Self::Part(_) | Self::Tuple(_) => None,
        }
    }
}

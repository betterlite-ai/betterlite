use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_openai::api::audio_file_input::decoder::{
    AudioDecodeLimits, AudioDecoder, NativeAudioDecoder,
};
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioStream, AudioTuple, AuthorizedAudioFiles, HEALTH_AUDIO_FILE_NAME,
    HEALTH_CHECK_AUDIO, TupleContent, calculate_request_duration, get_audio_file_content_hash,
    get_audio_file_name, normalize_language_to_bcp47, open_health_check_audio, process_audio_file,
};
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};

const HEALTH_DIRECTORY: &str = "/health";

#[derive(Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Process(FileCall),
    Hash(FileCall),
    Name(NameCall),
    Language(LanguageCall),
    Duration(FileCall),
    Health,
}

#[derive(Deserialize)]
struct FileCall {
    file: FileSpec,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct NameCall {
    file: FileSpec,
}

#[derive(Deserialize)]
struct LanguageCall {
    language: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum FileSpec {
    #[serde(rename = "bytes")]
    Bytes { content: String },
    #[serde(rename = "bytearray")]
    ByteArray { content: String },
    #[serde(rename = "text")]
    Text { content: String },
    #[serde(rename = "path")]
    Path { path: String },
    #[serde(rename = "stream")]
    Stream(StreamSpec),
    #[serde(rename = "tuple")]
    Tuple(TupleSpec),
    #[serde(rename = "unsupported")]
    Unsupported { class_name: String },
}

#[derive(Deserialize)]
struct TupleSpec {
    filename: Option<String>,
    #[serde(default)]
    content: Option<ContentSpec>,
    #[serde(rename = "mime")]
    _mime: Option<IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum ContentSpec {
    #[serde(rename = "bytes")]
    Bytes { content: String },
    #[serde(rename = "bytearray")]
    ByteArray { content: String },
    #[serde(rename = "text")]
    Text { content: String },
    #[serde(rename = "path")]
    Path { path: String },
    #[serde(rename = "stream")]
    Stream(StreamSpec),
    #[serde(rename = "unsupported")]
    Unsupported { class_name: String },
}

#[derive(Deserialize)]
struct StreamSpec {
    name: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    position: Option<u64>,
    #[serde(default)]
    read: ReadMode,
    #[serde(default)]
    seek: SeekMode,
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

struct FixtureStream {
    name: String,
    content: Vec<u8>,
    position: usize,
    read: ReadMode,
    seek: SeekMode,
}

impl FixtureStream {
    fn from_spec(spec: &StreamSpec) -> Result<Self, Box<crate::helpers::HelperError>> {
        let content = match &spec.content {
            Some(content) => BASE64.decode(content).map_err(failure)?,
            None => Vec::new(),
        };
        Ok(Self {
            name: spec.name.clone(),
            content,
            position: usize::try_from(spec.position.unwrap_or(0)).map_err(failure)?,
            read: spec.read,
            seek: spec.seek,
        })
    }

    fn fail_seek(&mut self, position: u64) -> io::Result<()> {
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
                let start = self.position.min(self.content.len());
                let content = self.content.get(start..).map(<[u8]>::to_vec);
                self.position = self.content.len();
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
            SeekMode::Works | SeekMode::Fails => Some(self.fail_seek(position)),
        }
    }
}

struct FixtureFiles(BTreeMap<PathBuf, Vec<u8>>);

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

enum PreparedContent {
    Bytes(Vec<u8>),
    ByteArray(Vec<u8>),
    Text(String),
    Path(PathBuf),
    Stream(FixtureStream),
    Unsupported(String),
}

impl PreparedContent {
    fn from_spec(spec: &ContentSpec) -> Result<Self, Box<crate::helpers::HelperError>> {
        Ok(match spec {
            ContentSpec::Bytes { content } => Self::Bytes(BASE64.decode(content).map_err(failure)?),
            ContentSpec::ByteArray { content } => {
                Self::ByteArray(BASE64.decode(content).map_err(failure)?)
            }
            ContentSpec::Text { content } => Self::Text(content.clone()),
            ContentSpec::Path { path } => Self::Path(PathBuf::from(path)),
            ContentSpec::Stream(stream) => Self::Stream(FixtureStream::from_spec(stream)?),
            ContentSpec::Unsupported { class_name } => Self::Unsupported(class_name.clone()),
        })
    }

    fn as_content(&mut self) -> TupleContent<'_> {
        match self {
            Self::Bytes(content) => TupleContent::Bytes(content.as_slice()),
            Self::ByteArray(content) => TupleContent::ByteArray(content.as_slice()),
            Self::Text(text) => TupleContent::Text(text.as_str()),
            Self::Path(path) => TupleContent::Path(path.as_path()),
            Self::Stream(stream) => TupleContent::Stream(stream),
            Self::Unsupported(class_name) => TupleContent::Unsupported {
                python_class: class_name.as_str(),
            },
        }
    }
}

enum PreparedFile {
    Bytes(Vec<u8>),
    ByteArray(Vec<u8>),
    Text(String),
    Path(PathBuf),
    Stream(FixtureStream),
    Tuple {
        filename: Option<String>,
        content: Option<PreparedContent>,
    },
    Unsupported(String),
}

impl PreparedFile {
    fn from_spec(spec: &FileSpec) -> Result<Self, Box<crate::helpers::HelperError>> {
        Ok(match spec {
            FileSpec::Bytes { content } => Self::Bytes(BASE64.decode(content).map_err(failure)?),
            FileSpec::ByteArray { content } => {
                Self::ByteArray(BASE64.decode(content).map_err(failure)?)
            }
            FileSpec::Text { content } => Self::Text(content.clone()),
            FileSpec::Path { path } => Self::Path(PathBuf::from(path)),
            FileSpec::Stream(stream) => Self::Stream(FixtureStream::from_spec(stream)?),
            FileSpec::Tuple(tuple) => Self::Tuple {
                filename: tuple.filename.clone(),
                content: match &tuple.content {
                    Some(content) => Some(PreparedContent::from_spec(content)?),
                    None => None,
                },
            },
            FileSpec::Unsupported { class_name } => Self::Unsupported(class_name.clone()),
        })
    }

    fn as_file(&mut self) -> AudioFile<'_> {
        match self {
            Self::Bytes(content) => AudioFile::Bytes(content.as_slice()),
            Self::ByteArray(content) => AudioFile::ByteArray(content.as_slice()),
            Self::Text(text) => AudioFile::Text(text.as_str()),
            Self::Path(path) => AudioFile::Path(path.as_path()),
            Self::Stream(stream) => AudioFile::Stream(stream),
            Self::Tuple { filename, content } => AudioFile::Tuple(AudioTuple {
                filename: filename.as_deref(),
                content: content.as_mut().map(PreparedContent::as_content),
            }),
            Self::Unsupported(class_name) => AudioFile::Unsupported {
                python_class: class_name.as_str(),
            },
        }
    }

    const fn stream_position(&self) -> Option<usize> {
        match self {
            Self::Stream(stream) => Some(stream.position),
            Self::Tuple { content, .. } => match content {
                Some(PreparedContent::Stream(stream)) => Some(stream.position),
                Some(_) | None => None,
            },
            Self::Bytes(_)
            | Self::ByteArray(_)
            | Self::Text(_)
            | Self::Path(_)
            | Self::Unsupported(_) => None,
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(outcome_of)
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_value(outcomes).map_err(failure)
    }
}

fn outcome_of(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    match call {
        Call::Process(file_call) => process_outcome(file_call),
        Call::Hash(file_call) => hash_outcome(file_call),
        Call::Name(name_call) => name_outcome(name_call),
        Call::Language(language_call) => Ok(language_outcome(language_call)),
        Call::Duration(file_call) => duration_outcome(file_call),
        Call::Health => health_outcome(),
    }
}

fn process_outcome(call: &FileCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let files = fixture_files(&call.files)?;
    let mut file = PreparedFile::from_spec(&call.file)?;
    Ok(match process_audio_file(file.as_file(), &files) {
        Ok(processed) => json!({
            "ok": true,
            "value": {
                "filename": processed.filename,
                "content_type": processed.content_type,
                "content": BASE64.encode(processed.file_content),
            },
        }),
        Err(_) => json!({ "ok": false }),
    })
}

fn hash_outcome(call: &FileCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let files = fixture_files(&call.files)?;
    let mut file = PreparedFile::from_spec(&call.file)?;
    let mut borrowed = file.as_file();
    let hash = get_audio_file_content_hash(&mut borrowed, &files);
    Ok(json!({
        "ok": true,
        "value": { "hash": hash, "position": file.stream_position() },
    }))
}

fn name_outcome(call: &NameCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut file = PreparedFile::from_spec(&call.file)?;
    let name = get_audio_file_name(&file.as_file());
    Ok(json!({ "ok": true, "value": name }))
}

fn language_outcome(call: &LanguageCall) -> Value {
    json!({
        "ok": true,
        "value": normalize_language_to_bcp47(&call.language),
    })
}

fn duration_outcome(call: &FileCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let files = fixture_files(&call.files)?;
    let mut file = PreparedFile::from_spec(&call.file)?;
    let decoder = NativeAudioDecoder {
        limits: AudioDecodeLimits::default(),
    };
    let mut borrowed = file.as_file();
    let seconds =
        calculate_request_duration(&mut borrowed, &files, AudioDecoder::Available(&decoder));
    Ok(json!({
        "ok": true,
        "value": {
            "duration": seconds.and_then(|decimal| decimal.to_f64()),
            "position": file.stream_position(),
        },
    }))
}

fn health_outcome() -> Result<Value, Box<crate::helpers::HelperError>> {
    let resource = Path::new(HEALTH_DIRECTORY).join(HEALTH_AUDIO_FILE_NAME);
    let mut entries = BTreeMap::new();
    entries.insert(resource, HEALTH_CHECK_AUDIO.to_vec());
    let files = FixtureFiles(entries);
    let mut handle =
        open_health_check_audio(&files, Path::new(HEALTH_DIRECTORY)).map_err(failure)?;
    let mut content = Vec::new();
    handle.read_to_end(&mut content).map_err(failure)?;
    Ok(json!({ "ok": true, "value": { "content": BASE64.encode(content) } }))
}

fn fixture_files(
    entries: &BTreeMap<String, String>,
) -> Result<FixtureFiles, Box<crate::helpers::HelperError>> {
    let mut files = BTreeMap::new();
    for (path, content) in entries {
        files.insert(
            PathBuf::from(path),
            BASE64.decode(content).map_err(failure)?,
        );
    }
    Ok(FixtureFiles(files))
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "AudioFileInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

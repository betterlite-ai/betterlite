use std::io::{self, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::api::file_type_records::{find_type, mime_from_type};

mod content;
pub mod decoder;
pub mod duration;
pub mod hashing;
pub mod language;

mod tests;

pub const HEALTH_AUDIO_FILE_NAME: &str = "audio_health_check.wav";

pub const HEALTH_CHECK_AUDIO: &[u8] = include_bytes!("audio_file_input/audio_health_check.wav");

const AUDIO_FILE_NAME: &str = "audio.wav";
const AUDIO_CONTENT_TYPE: &str = "audio/wav";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessedAudioFile {
    pub file_content: Vec<u8>,
    pub filename: String,
    pub content_type: String,
}

pub trait AudioStream {
    fn name(&self) -> Option<&str>;
    fn read(&mut self) -> io::Result<Option<Vec<u8>>>;
    fn tell(&mut self) -> Option<io::Result<u64>>;
    fn seek(&mut self, position: u64) -> Option<io::Result<()>>;
}

pub trait AuthorizedAudioFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>>;
}

pub enum AudioFile<'file> {
    Bytes(&'file [u8]),
    ByteArray(&'file [u8]),
    Stream(&'file mut dyn AudioStream),
    Tuple(AudioTuple<'file>),
    Path(&'file Path),
    Text(&'file str),
    Unsupported { python_class: &'file str },
}

pub struct AudioTuple<'file> {
    pub filename: Option<&'file str>,
    pub content: Option<TupleContent<'file>>,
}

pub enum TupleContent<'file> {
    Bytes(&'file [u8]),
    ByteArray(&'file [u8]),
    Text(&'file str),
    Path(&'file Path),
    Stream(&'file mut dyn AudioStream),
    Unsupported { python_class: &'file str },
}

#[derive(Debug, Error)]
pub enum AudioFileError {
    #[error(
        "process_audio_file does not accept bare str inputs. Pass bytes, an open file handle, a (filename, content) tuple, or a pathlib.Path."
    )]
    BareString,
    #[error(
        "process_audio_file does not accept bare str tuple contents. Pass bytes, an open file handle, or a pathlib.Path."
    )]
    BareStringTupleContent,
    #[error("Tuple must have at least 2 elements: (filename, content)")]
    ShortTuple,
    #[error("Unsupported content type in tuple: <class '{class}'>")]
    UnsupportedTupleContent { class: String },
    #[error("Unsupported audio_file type: <class '{class}'>")]
    UnsupportedInput { class: String },
    #[error("Could not extract file content from audio_file")]
    MissingContent,
    #[error("could not open {path}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read the audio file")]
    Read {
        #[source]
        source: io::Error,
    },
    #[error("could not seek the audio file")]
    Seek {
        #[source]
        source: io::Error,
    },
}

pub fn process_audio_file(
    file: AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> Result<ProcessedAudioFile, AudioFileError> {
    let (file_content, filename) = match file {
        AudioFile::Bytes(content) | AudioFile::ByteArray(content) => {
            (content.to_vec(), AUDIO_FILE_NAME.to_owned())
        }
        AudioFile::Text(_) => return Err(AudioFileError::BareString),
        AudioFile::Path(path) => (read_path(files, path)?, path_name(path)),
        AudioFile::Stream(stream) => {
            let filename = stream
                .name()
                .map_or_else(|| AUDIO_FILE_NAME.to_owned(), str::to_owned);
            (read_stream(stream)?, filename)
        }
        AudioFile::Tuple(tuple) => extract_tuple(tuple, files)?,
        AudioFile::Unsupported { python_class } => {
            return Err(AudioFileError::UnsupportedInput {
                class: python_class.to_owned(),
            });
        }
    };
    Ok(ProcessedAudioFile {
        content_type: content_type(&filename),
        filename,
        file_content,
    })
}

#[must_use]
pub fn get_audio_file_name(file: &AudioFile<'_>) -> String {
    match file {
        AudioFile::Stream(stream) => stream
            .name()
            .map_or_else(|| AUDIO_FILE_NAME.to_owned(), str::to_owned),
        AudioFile::Path(path) => path.to_str().unwrap_or_default().to_owned(),
        AudioFile::Text(text) => (*text).to_owned(),
        AudioFile::Tuple(tuple) => tuple_identity(tuple),
        AudioFile::Bytes(_) | AudioFile::ByteArray(_) => AUDIO_FILE_NAME.to_owned(),
        AudioFile::Unsupported { python_class } => python_class.to_string(),
    }
}

pub fn open_health_check_audio(
    files: &dyn AuthorizedAudioFiles,
    directory: &Path,
) -> Result<Box<dyn Read + Send>, AudioFileError> {
    let path = directory.join(HEALTH_AUDIO_FILE_NAME);
    match files.open(&path) {
        Ok(handle) => Ok(handle),
        Err(source) => Err(AudioFileError::Open { path, source }),
    }
}

fn extract_tuple(
    tuple: AudioTuple<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> Result<(Vec<u8>, String), AudioFileError> {
    let Some(content) = tuple.content else {
        return Err(AudioFileError::ShortTuple);
    };
    let filename = tuple
        .filename
        .filter(|filename| !filename.is_empty())
        .unwrap_or(AUDIO_FILE_NAME);
    let file_content = match content {
        TupleContent::Bytes(bytes) | TupleContent::ByteArray(bytes) => bytes.to_vec(),
        TupleContent::Text(_) => return Err(AudioFileError::BareStringTupleContent),
        TupleContent::Path(path) => read_path(files, path)?,
        TupleContent::Stream(stream) => read_stream(stream)?,
        TupleContent::Unsupported { python_class } => {
            return Err(AudioFileError::UnsupportedTupleContent {
                class: python_class.to_owned(),
            });
        }
    };
    Ok((file_content, filename.to_owned()))
}

fn read_path(files: &dyn AuthorizedAudioFiles, path: &Path) -> Result<Vec<u8>, AudioFileError> {
    files.read(path).map_err(|source| AudioFileError::Open {
        path: path.to_owned(),
        source,
    })
}

fn read_stream(stream: &mut dyn AudioStream) -> Result<Vec<u8>, AudioFileError> {
    let content = stream
        .read()
        .map_err(|source| AudioFileError::Read { source })?;
    if let Some(rewound) = stream.seek(0) {
        rewound.map_err(|source| AudioFileError::Seek { source })?;
    }
    content.ok_or(AudioFileError::MissingContent)
}

fn path_name(path: &Path) -> String {
    path.to_str()
        .and_then(|text| text.rsplit('/').next())
        .unwrap_or_default()
        .to_owned()
}

fn content_type(filename: &str) -> String {
    if filename.is_empty() {
        return AUDIO_CONTENT_TYPE.to_owned();
    }
    let extension = filename
        .rsplit_once('.')
        .map_or("wav", |(_, extension)| extension);
    find_type(extension).map_or_else(
        || AUDIO_CONTENT_TYPE.to_owned(),
        |kind| mime_from_type(kind).to_owned(),
    )
}

fn tuple_identity(tuple: &AudioTuple<'_>) -> String {
    let name = tuple
        .filename
        .filter(|name| !name.is_empty())
        .unwrap_or("None");
    tuple.content.as_ref().map_or_else(
        || format!("({name},)"),
        |content| format!("({name}, {})", content_kind(content)),
    )
}

fn content_kind(content: &TupleContent<'_>) -> String {
    match content {
        TupleContent::Bytes(_) => "bytes".to_owned(),
        TupleContent::ByteArray(_) => "bytearray".to_owned(),
        TupleContent::Text(_) => "text".to_owned(),
        TupleContent::Path(_) => "path".to_owned(),
        TupleContent::Stream(stream) => stream.name().unwrap_or("stream").to_owned(),
        TupleContent::Unsupported { python_class } => python_class.to_string(),
    }
}

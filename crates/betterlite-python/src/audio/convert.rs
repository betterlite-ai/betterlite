pub(crate) mod speech;
mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioFileError, AudioTuple, AuthorizedAudioFiles, ProcessedAudioFile, TupleContent,
    get_audio_file_name, process_audio_file,
};
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_sdk::call_files::{CallFile, CallFilesError, files_from_payload};
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, TryFromFloatSecsError};

pub(crate) const OPENAI_PROVIDER: &str = "openai";
pub(crate) const UNNAMED_FILE: &str = "file";
pub(crate) const DEFAULT_TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const DEFAULT_MAX_RETRIES: u32 = 2;

// The five transcription params LiteLLM maps from its named arguments, in its own order.
const TRANSCRIPTION_FIELDS: [&str; 5] = [
    "language",
    "prompt",
    "response_format",
    "temperature",
    "timestamp_granularities",
];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AudioSettings {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) organization: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) num_retries: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AudioCallWire {
    pub(crate) arguments: RequestMap,
    pub(crate) settings: AudioSettings,
    pub(crate) atranscription: bool,
    pub(crate) file_from_kwargs: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SpeechCallWire {
    pub(crate) arguments: RequestMap,
    pub(crate) settings: AudioSettings,
    pub(crate) aspeech: bool,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Files(#[from] CallFilesError),
    #[error(transparent)]
    Audio(#[from] AudioFileError),
    #[error("the timeout is not a number of seconds: {source}")]
    Timeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("{name} must be {expected}")]
    Field {
        name: String,
        expected: &'static str,
    },
}

#[derive(Serialize)]
pub(crate) struct TranscriptionAnswer {
    #[serde(flatten)]
    pub(crate) reply: TranscriptionResponse,
    pub(crate) response_ms: f64,
    pub(crate) hidden_params: IndexMap<String, Json>,
}

#[derive(Serialize)]
pub(crate) struct SpeechHeader {
    pub(crate) status_code: u16,
    pub(crate) headers: IndexMap<String, String>,
}

pub(crate) struct SpeechAnswer {
    pub(crate) content: Vec<u8>,
    pub(crate) header: SpeechHeader,
}

pub(crate) fn parse(
    text: &str,
    blobs: &mut [Vec<u8>],
) -> Result<(AudioCallWire, Vec<(String, CallFile)>), ConvertError> {
    let wire: AudioCallWire =
        serde_json::from_str(text).map_err(|source| ConvertError::Json { source })?;
    let files = files_from_payload(&wire.arguments, blobs)?;
    Ok((wire, files))
}

pub(crate) fn parse_speech(text: &str) -> Result<SpeechCallWire, ConvertError> {
    let wire: SpeechCallWire =
        serde_json::from_str(text).map_err(|source| ConvertError::Json { source })?;
    Ok(wire)
}

// process_audio_file rejects a bare str file, which reaches here as a JSON string.
pub(crate) fn bare_string_file(arguments: &RequestMap) -> Result<(), ConvertError> {
    match arguments.get("file") {
        Some(Json::Text(_)) => Err(ConvertError::Audio(AudioFileError::BareString)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) | Json::Null)
        | None => Ok(()),
    }
}

// process_audio_file: a name's extension picks the type, and a file without a name becomes audio.wav.
pub(crate) fn audio_file(file: &CallFile) -> Result<(ProcessedAudioFile, String), AudioFileError> {
    let unnamed = file.name.is_empty() || file.name == UNNAMED_FILE;
    let audio = if unnamed {
        AudioFile::Bytes(&file.bytes)
    } else {
        AudioFile::Tuple(AudioTuple {
            filename: &file.name,
            content: Some(TupleContent::Bytes(&file.bytes)),
        })
    };
    let input = get_audio_file_name(&audio);
    let processed = process_audio_file(audio, &NoFiles)?;
    let processed = match &file.content_type {
        Some(kind) => ProcessedAudioFile {
            content_type: kind.clone(),
            ..processed
        },
        None => processed,
    };
    Ok((processed, input))
}

pub(crate) fn first_file(files: &[(String, CallFile)]) -> Option<&CallFile> {
    files
        .iter()
        .find(|(name, _file)| name == "file")
        .map(|(_name, file)| file)
}

// get_llm_provider's openai config: gpt-4o transcribes with its own family, everything else with whisper's.
pub(crate) fn family_for(model: &str) -> TranscriptionFamily {
    if model.contains("gpt-4o") {
        TranscriptionFamily::Gpt
    } else {
        TranscriptionFamily::Whisper
    }
}

pub(crate) fn transcription_options(arguments: &RequestMap) -> IndexMap<String, Json> {
    TRANSCRIPTION_FIELDS
        .iter()
        .filter_map(|name| {
            arguments
                .get(*name)
                .filter(|value| !matches!(value, Json::Null))
                .map(|value| ((*name).to_owned(), value.clone()))
        })
        .collect()
}

pub(crate) fn text(arguments: &RequestMap, name: &str) -> Result<Option<String>, ConvertError> {
    match arguments.get(name) {
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Null) | None => Ok(None),
        Some(_other) => Err(field(name, "text")),
    }
}

pub(crate) fn timeout(arguments: &RequestMap) -> Result<Duration, ConvertError> {
    match arguments.get("timeout") {
        Some(Json::Number(value)) => {
            let seconds = value.as_f64().ok_or_else(|| field("timeout", "a number"))?;
            Duration::try_from_secs_f64(seconds).map_err(|source| ConvertError::Timeout { source })
        }
        None | Some(Json::Null) => Ok(DEFAULT_TRANSCRIPTION_TIMEOUT),
        Some(_other) => Err(field("timeout", "a number")),
    }
}

pub(crate) fn max_retries(arguments: &RequestMap) -> Result<Option<u32>, ConvertError> {
    match arguments.get("max_retries") {
        Some(Json::Number(value)) => {
            let given = value
                .as_i64()
                .ok_or_else(|| field("max_retries", "an integer"))?;
            Ok(Some(u32::try_from(given.max(0)).unwrap_or(u32::MAX)))
        }
        None | Some(Json::Null) => Ok(None),
        Some(_other) => Err(field("max_retries", "an integer")),
    }
}

pub(crate) fn field(name: &str, expected: &'static str) -> ConvertError {
    ConvertError::Field {
        name: name.to_owned(),
        expected,
    }
}

// A file reaches Rust as bytes, so the audio path never opens a path itself.
pub(crate) struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }
}

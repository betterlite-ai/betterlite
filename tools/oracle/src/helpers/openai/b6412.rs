use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, ProcessedAudioFile, TupleContent,
    process_audio_file,
};
use betterlite_openai::openai_base_config::Object as WireObject;
use betterlite_prompts::arguments::json::{Json, Object as JsonObject};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::other("the helper reads no files"))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::other("the helper opens no files"))
    }
}

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ChatRequest {
        requests: Vec<ChatRequestCall>,
    },
    Chunks {
        chunks: Vec<JsonObject>,
    },
    Embedding {
        #[serde(default)]
        headers: Vec<HeaderCall>,
        #[serde(default)]
        requests: Vec<EmbeddingCall>,
        #[serde(default)]
        replies: Vec<ReplyCall>,
    },
    Transcription {
        provider: AudioProvider,
        #[serde(default)]
        requests: Vec<TranscriptionCall>,
        #[serde(default)]
        replies: Vec<ReplyCall>,
    },
    ScalewayKey {
        #[serde(default)]
        calls: Vec<HeaderCall>,
        #[serde(default)]
        replies: Vec<ReplyCall>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HeaderProvider {
    OvhcloudEmbedding,
    OvhcloudAudio,
    ScalewayAudio,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AudioProvider {
    Ovhcloud,
    Scaleway,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatRequestCall {
    api_base: Option<String>,
    model: String,
    optional_params: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderCall {
    api_key: Option<String>,
    headers: BTreeMap<String, String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingCall {
    model: String,
    input: Json,
    // the embedding config advertises no supported param, so this is only here to prove nothing is copied
    #[serde(rename = "non_default_params")]
    _non_default_params: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    body: String,
    status: u16,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TranscriptionCall {
    api_base: Option<String>,
    model: String,
    optional_params: WireObject,
    filename: String,
    content: String,
    env: BTreeMap<String, String>,
}

impl HeaderProvider {
    const fn env_name(self) -> &'static str {
        match self {
            Self::ScalewayAudio => "SCW_SECRET_KEY",
            Self::OvhcloudEmbedding | Self::OvhcloudAudio => "OVHCLOUD_API_KEY",
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(match input {
            Input::ChatRequest { requests } => {
                Value::Array(requests.iter().map(chat_request_value).collect())
            }
            Input::Chunks { chunks } => Value::Array(chunks.iter().map(chunk_value).collect()),
            Input::Embedding {
                headers,
                requests,
                replies,
            } => json!({
                "headers": headers
                    .iter()
                    .map(|call| header_value(HeaderProvider::OvhcloudEmbedding, call))
                    .collect::<Vec<Value>>(),
                "requests": requests
                    .iter()
                    .map(embedding_request_value)
                    .collect::<Vec<Value>>(),
                "replies": replies
                    .iter()
                    .map(embedding_reply_value)
                    .collect::<Vec<Value>>(),
            }),
            Input::Transcription {
                provider,
                requests,
                replies,
            } => json!({
                "requests": requests
                    .iter()
                    .map(|call| transcription_request_value(*provider, call))
                    .collect::<Vec<Value>>(),
                "replies": replies
                    .iter()
                    .map(|reply| transcription_reply_value(*provider, reply))
                    .collect::<Vec<Value>>(),
            }),
            Input::ScalewayKey { calls, replies } => json!({
                "headers": calls
                    .iter()
                    .map(|call| header_value(HeaderProvider::ScalewayAudio, call))
                    .collect::<Vec<Value>>(),
                "replies": replies
                    .iter()
                    .map(|reply| transcription_reply_value(AudioProvider::Scaleway, reply))
                    .collect::<Vec<Value>>(),
            }),
        })
    }
}

fn pairs(body: &JsonObject) -> Value {
    Value::Array(
        body.iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}

fn env_key(env: &BTreeMap<String, String>, name: &str) -> Option<SecretString> {
    env.get(name)
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}

mod wire;
use self::wire::{
    chat_request_value, chunk_value, embedding_reply_value, embedding_request_value, header_value,
    transcription_reply_value, transcription_request_value,
};
pub(super) fn processed(call: &TranscriptionCall) -> Result<ProcessedAudioFile, Value> {
    let file = AudioFile::Tuple(AudioTuple {
        filename: &call.filename,
        content: Some(TupleContent::Bytes(call.content.as_bytes())),
    });
    process_audio_file(file, &NoFiles).map_err(|error| {
        json!({
            "url": call.api_base,
            "ok": false,
            "message": error.to_string(),
        })
    })
}

pub(super) fn file_value(file: &ProcessedAudioFile) -> Value {
    json!({
        "filename": file.filename,
        "content": String::from_utf8_lossy(&file.file_content),
        "content_type": file.content_type,
    })
}

pub(super) fn fields_pairs(fields: &[(String, Value)]) -> Value {
    Value::Array(
        fields
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}

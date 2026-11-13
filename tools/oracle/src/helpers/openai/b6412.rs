use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, ProcessedAudioFile, TupleContent,
    process_audio_file,
};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::openai_base_config::{Headers, Object as WireObject};
use betterlite_openai::ovhcloud_audio::reply::decode_transcription_reply as ovhcloud_reply;
use betterlite_openai::ovhcloud_audio::request::{
    complete_url as ovhcloud_audio_url, transcription_request as ovhcloud_form,
    validate_environment as ovhcloud_audio_headers,
};
use betterlite_openai::ovhcloud_chat::chunk::chunk_parser;
use betterlite_openai::ovhcloud_chat::request::{
    ChatTransform, complete_url as chat_url, transform_request,
};
use betterlite_openai::ovhcloud_embedding::reply::decode_embedding_reply;
use betterlite_openai::ovhcloud_embedding::request::{
    complete_url as embedding_url, transform_embedding_request,
    validate_environment as embedding_headers,
};
use betterlite_openai::scaleway_audio::reply::decode_transcription_reply as scaleway_reply;
use betterlite_openai::scaleway_audio::request::{
    complete_url as scaleway_url, transcription_request as scaleway_form,
    validate_environment as scaleway_headers,
};
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

#[derive(Debug, Clone, Copy, Deserialize)]
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

fn chat_request_value(call: &ChatRequestCall) -> Value {
    let messages: &[ChatMessage] = &[];
    let environment = BTreeMap::<String, String>::new();
    let transform = ChatTransform {
        model: &call.model,
        messages,
        optional_params: &call.optional_params,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let url = chat_url(call.api_base.as_deref());
    match transform_request(&transform) {
        Ok(body) => json!({"url": url, "ok": true, "value": body}),
        Err(_) => json!({"url": url, "ok": false}),
    }
}

fn chunk_value(chunk: &JsonObject) -> Value {
    match chunk_parser(chunk) {
        Ok(parsed) => json!({"ok": true, "value": parsed}),
        Err(error) => json!({"ok": false, "status": error.status_code}),
    }
}

fn header_value(provider: HeaderProvider, call: &HeaderCall) -> Value {
    let explicit = call.api_key.clone().map(SecretString::from);
    let from_env = match provider {
        HeaderProvider::ScalewayAudio => env_key(&call.env, "SCW_SECRET_KEY"),
        HeaderProvider::OvhcloudEmbedding | HeaderProvider::OvhcloudAudio => {
            env_key(&call.env, "OVHCLOUD_API_KEY")
        }
    };
    match provider {
        HeaderProvider::OvhcloudEmbedding => json!({
            "ok": true,
            "headers": embedding_headers(&call.headers, explicit.as_ref(), from_env.as_ref()),
        }),
        HeaderProvider::OvhcloudAudio => json!({
            "ok": true,
            "headers": ovhcloud_audio_headers(&call.headers, explicit.as_ref(), from_env.as_ref()),
        }),
        HeaderProvider::ScalewayAudio => {
            match scaleway_headers(&call.headers, explicit.as_ref(), from_env.as_ref()) {
                Ok(headers) => json!({"ok": true, "headers": headers}),
                Err(error) => json!({"ok": false, "status": error.status_code}),
            }
        }
    }
}

fn embedding_request_value(call: &EmbeddingCall) -> Value {
    let body = transform_embedding_request(&call.model, &call.input, &JsonObject::new());
    json!({"value": pairs(&body)})
}

fn embedding_reply_value(call: &ReplyCall) -> Value {
    match decode_embedding_reply(call.body.as_bytes(), call.status, &call.headers) {
        Ok(reply) => json!({"ok": true, "value": reply}),
        Err(error) => json!({"ok": false, "status": error.status_code}),
    }
}

fn processed(call: &TranscriptionCall) -> Result<ProcessedAudioFile, Value> {
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

fn file_value(file: &ProcessedAudioFile) -> Value {
    json!({
        "filename": file.filename,
        "content": String::from_utf8_lossy(&file.file_content),
        "content_type": file.content_type,
    })
}

fn fields_pairs(fields: &[(String, Value)]) -> Value {
    Value::Array(
        fields
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}

fn transcription_request_value(provider: AudioProvider, call: &TranscriptionCall) -> Value {
    let file = match processed(call) {
        Ok(file) => file,
        Err(failure) => return failure,
    };
    match provider {
        AudioProvider::Ovhcloud => {
            let from_env = env_key(&call.env, "OVHCLOUD_API_KEY");
            let headers = ovhcloud_audio_headers(&Headers::new(), None, from_env.as_ref());
            let request = ovhcloud_form(&call.model, &file, &call.optional_params);
            json!({
                "url": ovhcloud_audio_url(call.api_base.as_deref()),
                "ok": true,
                "headers": headers,
                "form": fields_pairs(&request.fields),
                "file": file_value(&request.file),
            })
        }
        AudioProvider::Scaleway => {
            let from_env = env_key(&call.env, "SCW_SECRET_KEY");
            let url = scaleway_url(call.api_base.as_deref());
            let headers = match scaleway_headers(&Headers::new(), None, from_env.as_ref()) {
                Ok(headers) => headers,
                Err(error) => return json!({"url": url, "ok": false, "status": error.status_code}),
            };
            let request = scaleway_form(&call.model, &file, &call.optional_params);
            json!({
                "url": url,
                "ok": true,
                "headers": headers,
                "form": fields_pairs(&request.fields),
                "file": file_value(&request.file),
            })
        }
    }
}

fn transcription_reply_value(provider: AudioProvider, call: &ReplyCall) -> Value {
    match provider {
        AudioProvider::Ovhcloud => {
            match ovhcloud_reply(call.body.as_bytes(), call.status, &call.headers) {
                Ok(transcription) => {
                    json!({"ok": true, "value": {"text": transcription.response.text}})
                }
                Err(error) => json!({"ok": false, "status": error.status_code}),
            }
        }
        AudioProvider::Scaleway => {
            match scaleway_reply(call.body.as_bytes(), call.status, &call.headers) {
                Ok(transcription) => {
                    let mut value = json!({"text": transcription.response.text});
                    for name in ["segments", "language"] {
                        if let Some(field) = transcription.response.extra.get(name) {
                            value[name] = json!(field);
                        }
                    }
                    json!({"ok": true, "value": value})
                }
                Err(error) => json!({"ok": false, "status": error.status_code}),
            }
        }
    }
}

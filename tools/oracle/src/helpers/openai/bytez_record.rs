use std::collections::BTreeMap;

use betterlite_openai::bytez_wire::environment::{complete_url, validate_environment};
use betterlite_openai::bytez_wire::messages::{
    RawMessage, get_tokens_from_messages, normalize_messages,
};
use betterlite_openai::bytez_wire::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_openai::bytez_wire::reply::{ReplyInput, decode_bytez_reply};
use betterlite_openai::bytez_wire::request::request_body;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::bytez_stream_call::error::BytezStreamError;
use betterlite_provider::bytez_stream_call::stream::BytezCustomStream;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, StreamChunk};
use futures_util::StreamExt as _;
use futures_util::stream::empty;
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Options {
        #[serde(rename = "supported_model")]
        _supported_model: String,
        #[serde(default)]
        mappings: Vec<Mapping>,
    },
    Request {
        #[serde(default)]
        requests: Vec<RequestEntry>,
    },
    Usage {
        #[serde(default)]
        messages: Vec<RawMessage>,
    },
    Reply {
        #[serde(default)]
        requests: Vec<ReplyEntry>,
    },
    Chunks {
        model: String,
        provider: String,
        #[serde(default)]
        fragments: Vec<String>,
    },
}

#[derive(Deserialize)]
pub(crate) struct Mapping {
    #[serde(rename = "non_default_params")]
    supplied: Object,
    #[serde(default, rename = "optional_params")]
    existing: Object,
    #[serde(default, rename = "drop_params")]
    drop: bool,
}

#[derive(Deserialize)]
pub(crate) struct RequestEntry {
    model: String,
    #[serde(default)]
    messages: Vec<RawMessage>,
    #[serde(default)]
    optional_params: Object,
    api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct ReplyEntry {
    body: String,
    status: u16,
    model: String,
    created: i64,
    #[serde(default)]
    messages: Vec<RawMessage>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(match input {
            Input::Options { mappings, .. } => json!({
                "supported": SUPPORTED_OPENAI_PARAMS,
                "mapped": mappings.iter().map(mapped).collect::<Vec<Value>>(),
            }),
            Input::Request { requests } => Value::Array(requests.iter().map(request).collect()),
            Input::Usage { messages } => usage(messages),
            Input::Reply { requests } => Value::Array(requests.iter().map(reply).collect()),
            Input::Chunks {
                model,
                provider,
                fragments,
            } => chunks(model, provider, fragments),
        })
    }
}

fn mapped(mapping: &Mapping) -> Value {
    match map_openai_params(&mapping.supplied, mapping.existing.clone(), mapping.drop) {
        Ok(adapted) => json!({"ok": true, "value": adapted}),
        Err(_error) => json!({"ok": false}),
    }
}

fn usage(messages: &[RawMessage]) -> Value {
    let Ok(normalized) = normalize_messages(messages) else {
        return json!({"ok": false, "status": Value::Null});
    };
    match get_tokens_from_messages(&normalized) {
        Ok(prompt_tokens) => json!({
            "ok": true,
            "value": {"normalized": normalized, "prompt_tokens": prompt_tokens},
        }),
        Err(_error) => json!({"ok": false, "status": Value::Null}),
    }
}

fn request(entry: &RequestEntry) -> Value {
    let api_key = entry.api_key.clone().map(SecretString::from);
    let Ok(validated) =
        validate_environment(entry.headers.clone(), &entry.messages, api_key.as_ref())
    else {
        return json!({"ok": false, "status": Value::Null});
    };
    let Ok(url) = complete_url(&entry.model) else {
        return json!({"ok": false, "status": Value::Null});
    };
    match request_body(&entry.messages, &entry.optional_params) {
        Ok(body) => json!({"ok": true, "value": {"url": url, "headers": validated, "body": body}}),
        Err(_error) => json!({"ok": false, "status": Value::Null}),
    }
}

fn reply(entry: &ReplyEntry) -> Value {
    let headers = header_map(&entry.headers);
    let input = ReplyInput {
        body: entry.body.as_bytes(),
        status: entry.status,
        model: &entry.model,
        created: entry.created,
        messages: &entry.messages,
        headers: &headers,
    };
    match decode_bytez_reply(&input) {
        Ok(reply) => json!({"ok": true, "value": reply}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn chunks(model: &str, provider: &str, fragments: &[String]) -> Value {
    let Ok(redactor) = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH) else {
        return json!({"failed": true});
    };
    let mut stream = BytezCustomStream {
        shaper: ChunkShaper::new(
            model.to_owned(),
            Some(provider.to_owned()),
            false,
            JsonObject::new(),
        ),
        repeated_chunk_limit: 100,
        fragments: empty().boxed(),
        redactor: &redactor,
    };
    Value::Array(
        fragments
            .iter()
            .map(|fragment| chunk(&mut stream, fragment))
            .collect(),
    )
}

fn chunk(stream: &mut BytezCustomStream<'_>, fragment: &str) -> Value {
    match stream.chunk_creator(fragment) {
        Ok(Some(chunk)) => json!({"chunk": projection(&chunk)}),
        Ok(None) => json!({"chunk": Value::Null}),
        Err(BytezStreamError::StopIteration) => json!({"end": true}),
        Err(_error) => json!({"failed": true}),
    }
}

fn projection(chunk: &StreamChunk) -> Value {
    let choice = chunk.choices.first();
    json!({
        "content": choice.and_then(|first| first.delta.content.clone()),
        "role": choice.and_then(|first| first.delta.role.clone()),
        "finish_reason": choice.and_then(|first| first.finish_reason.clone()),
        "index": choice.map(|first| first.index),
        "model": chunk.model.clone(),
        "object": chunk.object.clone(),
        "id_prefix": chunk.id.starts_with("chatcmpl-"),
    })
}

fn header_map(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (raw_name, raw_value) in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(raw_name.as_bytes()),
            HeaderValue::from_str(raw_value),
        ) {
            map.insert(name, value);
        }
    }
    map
}

use axum::body::Body;
use betterlite_mcp::wire_compat::capabilities::policy::{ClientInfo, InitializeParams};
use betterlite_prompts::arguments::json::Json;
use bytes::Bytes;
use futures_util::stream::{self, StreamExt as _};
use serde::Deserialize;

mod tests;

pub const PEEK_LIMIT: usize = 4096;

const INITIALIZE_METHOD: &str = "initialize";
const JSONRPC_VERSION: &str = "2.0";
const METHOD_KEY: &str = "method";

// _read_request_body_for_routing: keep the first PEEK_LIMIT bytes for sniffing, replay every frame and the rest
pub async fn peek_routing_body(body: Body) -> (Body, Bytes) {
    let mut source = body.into_data_stream();
    let mut frames: Vec<Result<Bytes, axum::Error>> = Vec::new();
    let mut sniffed: Vec<u8> = Vec::new();
    while sniffed.len() < PEEK_LIMIT {
        let Some(frame) = source.next().await else {
            break;
        };
        match frame {
            Ok(chunk) => {
                let remaining = PEEK_LIMIT.saturating_sub(sniffed.len());
                if let Some(kept) = chunk.get(..chunk.len().min(remaining)) {
                    sniffed.extend_from_slice(kept);
                }
                frames.push(Ok(chunk));
            }
            Err(error) => {
                frames.push(Err(error));
                break;
            }
        }
    }
    let replay = stream::iter(frames).chain(source);
    (Body::from_stream(replay), Bytes::from(sniffed))
}

pub fn lossy_utf8(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}

pub fn is_initialize_request(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Json>(bytes) else {
        return false;
    };
    value
        .as_object()
        .and_then(|fields| fields.get(METHOD_KEY))
        .and_then(Json::as_str)
        .is_some_and(|method| method == INITIALIZE_METHOD)
}

#[derive(Deserialize)]
struct InitializeEnvelope {
    #[serde(default = "default_method")]
    method: String,
    params: InitializeParams,
}

// pydantic's InitializeRequest defaults method to "initialize" while a Literal rejects every other value
fn default_method() -> String {
    INITIALIZE_METHOD.to_owned()
}

pub fn initialize_client_info(bytes: &[u8]) -> Option<ClientInfo> {
    let envelope: InitializeEnvelope = serde_json::from_slice(bytes).ok()?;
    if envelope.method != INITIALIZE_METHOD {
        return None;
    }
    Some(envelope.params.client_info)
}

pub fn has_top_level_method(text: &str) -> bool {
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut containers: Vec<bool> = Vec::new();
    let mut reading_key = false;
    let mut expect_key = false;
    let mut key_chars: Vec<char> = Vec::new();
    for character in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
                if reading_key && depth == 1 && key_chars.iter().copied().eq(METHOD_KEY.chars()) {
                    return true;
                }
            } else if reading_key {
                key_chars.push(character);
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                reading_key =
                    expect_key && depth >= 1 && containers.last().copied().unwrap_or(false);
                key_chars.clear();
                expect_key = false;
            }
            '{' | '[' => {
                depth = depth.saturating_add(1);
                containers.push(character == '{');
                expect_key = character == '{';
            }
            '}' | ']' => {
                containers.pop();
                depth = depth.saturating_sub(1);
                if depth <= 0 {
                    break;
                }
                expect_key = false;
            }
            ',' => {
                expect_key = containers.last().copied().unwrap_or(false);
            }
            ':' => {
                expect_key = false;
            }
            _ => {}
        }
    }
    false
}

// the inline test in handle_streamable_http_mcp: a parse failure means the peek truncated the body
pub fn is_jsonrpc_response(bytes: &[u8]) -> bool {
    match serde_json::from_slice::<Json>(bytes) {
        Ok(value) => value.as_object().is_some_and(|fields| {
            fields.get("jsonrpc").and_then(Json::as_str) == Some(JSONRPC_VERSION)
                && fields.contains_key("id")
                && !fields.contains_key(METHOD_KEY)
                && (fields.contains_key("result") || fields.contains_key("error"))
        }),
        Err(_) => {
            let text = lossy_utf8(bytes);
            text.contains("\"jsonrpc\"")
                && (text.contains("\"result\"") || text.contains("\"error\""))
                && !has_top_level_method(&text)
        }
    }
}

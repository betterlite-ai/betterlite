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

#[must_use]
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

#[must_use]
pub fn initialize_client_info(bytes: &[u8]) -> Option<ClientInfo> {
    let envelope: InitializeEnvelope = serde_json::from_slice(bytes).ok()?;
    if envelope.method != INITIALIZE_METHOD {
        return None;
    }
    Some(envelope.params.client_info)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanMode {
    Outside,
    Key,
    Value,
    KeyEscape,
    ValueEscape,
}

struct ScanState {
    depth: i32,
    mode: ScanMode,
    containers: Vec<bool>,
    expect_key: bool,
    key_chars: Vec<char>,
}

fn quoted_step(mode: &mut ScanMode, character: char, key_chars: &mut Vec<char>) -> bool {
    match *mode {
        ScanMode::Key => match character {
            '"' => {
                *mode = ScanMode::Outside;
                return true;
            }
            '\\' => *mode = ScanMode::KeyEscape,
            _ => key_chars.push(character),
        },
        ScanMode::Value => match character {
            '"' => *mode = ScanMode::Outside,
            '\\' => *mode = ScanMode::ValueEscape,
            _ => {}
        },
        ScanMode::KeyEscape => *mode = ScanMode::Key,
        ScanMode::ValueEscape => *mode = ScanMode::Value,
        ScanMode::Outside => {}
    }
    false
}

// one character outside a string: opens or closes the containers and tracks whether a key may start
fn structural_step(scan: &mut ScanState, character: char) -> bool {
    match character {
        '"' => {
            scan.mode = if scan.expect_key && scan.depth >= 1_i32 {
                ScanMode::Key
            } else {
                ScanMode::Value
            };
            scan.key_chars.clear();
            scan.expect_key = false;
        }
        '{' | '[' => {
            scan.depth = scan.depth.saturating_add(1);
            scan.containers.push(character == '{');
            scan.expect_key = character == '{';
        }
        '}' | ']' => {
            scan.containers.pop();
            scan.depth = scan.depth.saturating_sub(1);
            if scan.depth <= 0_i32 {
                return false;
            }
            scan.expect_key = false;
        }
        ',' => {
            scan.expect_key = scan.containers.last().copied().unwrap_or(false);
        }
        ':' => {
            scan.expect_key = false;
        }
        _ => {}
    }
    true
}

#[must_use]
pub fn has_top_level_method(text: &str) -> bool {
    let mut scan = ScanState {
        depth: 0_i32,
        mode: ScanMode::Outside,
        containers: Vec::new(),
        expect_key: false,
        key_chars: Vec::new(),
    };
    for character in text.chars() {
        if scan.mode != ScanMode::Outside {
            if quoted_step(&mut scan.mode, character, &mut scan.key_chars)
                && scan.depth == 1_i32
                && scan.key_chars.iter().copied().eq(METHOD_KEY.chars())
            {
                return true;
            }
            continue;
        }
        if !structural_step(&mut scan, character) {
            break;
        }
    }
    false
}

// the inline test in handle_streamable_http_mcp: a parse failure means the peek truncated the body
#[must_use]
pub fn is_jsonrpc_response(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Json>(bytes).map_or_else(
        |_| {
            let text = lossy_utf8(bytes);
            text.contains("\"jsonrpc\"")
                && (text.contains("\"result\"") || text.contains("\"error\""))
                && !has_top_level_method(&text)
        },
        |value| {
            value.as_object().is_some_and(|fields| {
                fields.get("jsonrpc").and_then(Json::as_str) == Some(JSONRPC_VERSION)
                    && fields.contains_key("id")
                    && !fields.contains_key(METHOD_KEY)
                    && (fields.contains_key("result") || fields.contains_key("error"))
            })
        },
    )
}

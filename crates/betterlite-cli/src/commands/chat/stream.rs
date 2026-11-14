use std::str::from_utf8;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::{ReadError, check_headers, read_bounded};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use tokio::time::timeout;

use crate::commands::{BODY_LIMIT_BYTES, Client, build_headers};

use super::stream_line::{LineEvent, StreamLines};
use super::{ChatError, Message, Terminal};

const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";
const STREAM_TIMEOUT: Duration = Duration::from_secs(600);
const CONTENT_TYPE: (&str, &str) = ("Content-Type", "application/json");

pub(super) struct StreamRequest<'request> {
    pub(super) model: &'request str,
    pub(super) messages: &'request [Message],
    pub(super) temperature: f64,
    pub(super) max_tokens: Option<i64>,
}

impl StreamRequest<'_> {
    fn encode(&self) -> Result<Vec<u8>, ChatError> {
        serde_json::to_vec(&CompletionsBody {
            model: self.model,
            messages: self.messages,
            stream: true,
            temperature: self.temperature,
            max_tokens: self.max_tokens,
        })
        .map_err(ChatError::Encode)
    }
}

#[derive(Serialize)]
struct CompletionsBody<'body> {
    model: &'body str,
    messages: &'body [Message],
    stream: bool,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct StreamErrorBody {
    #[serde(default)]
    error: Option<StreamErrorDetail>,
}

#[derive(Deserialize)]
struct StreamErrorDetail {
    #[serde(default)]
    message: Option<String>,
}

pub(super) fn stream_reply(
    client: &Client,
    terminal: &mut dyn Terminal,
    request: &StreamRequest<'_>,
) -> Result<String, ChatError> {
    match open_and_read(client, terminal, request) {
        Ok(content) => Ok(content),
        Err(error) => {
            terminal.write_error_line(&format!("Error: {error}"))?;
            Ok(String::new())
        }
    }
}

fn block_on_timeout<Value>(
    client: &Client,
    future: impl Future<Output = Value>,
) -> Result<Value, ChatError> {
    // tokio::time::timeout reads the current runtime when it is created
    let _entered = client.runtime.enter();
    client
        .runtime
        .block_on(timeout(STREAM_TIMEOUT, future))
        .map_err(|_elapsed| ChatError::Timeout {
            seconds: STREAM_TIMEOUT.as_secs(),
        })
}

// the streamed response type cannot be named outside betterlite-http, so opening it and reading it share this function
fn open_and_read(
    client: &Client,
    terminal: &mut dyn Terminal,
    request: &StreamRequest<'_>,
) -> Result<String, ChatError> {
    let headers = build_headers(
        vec![(String::from(CONTENT_TYPE.0), String::from(CONTENT_TYPE.1))],
        client.api_key.as_ref(),
    )?;
    let built = client
        .http
        .client
        .post(client.resolve(CHAT_COMPLETIONS_PATH)?)
        .headers(headers)
        .body(request.encode()?)
        .build()
        .map_err(HttpError::Request)?;
    let mut response = block_on_timeout(client, client.http.execute(built))??;
    let status = response.status().as_u16();
    if status == 401 {
        return Err(ChatError::Status { status });
    }
    if status >= 400 {
        let read = block_on_timeout(client, read_bounded(&mut response, BODY_LIMIT_BYTES))?;
        return report_http_error(terminal, status, read);
    }
    let limit = client.http.limits.header_bytes;
    check_headers(response.headers(), limit).map_err(|_too_large| ChatError::Headers { limit })?;
    let content = read_events(terminal, || {
        block_on_timeout(client, response.chunk())?
            .map_err(|source| ChatError::Http(HttpError::Request(source)))
    })?;
    terminal.write_line("")?;
    Ok(content)
}

fn read_events<Chunk: AsRef<[u8]>>(
    terminal: &mut dyn Terminal,
    mut next_chunk: impl FnMut() -> Result<Option<Chunk>, ChatError>,
) -> Result<String, ChatError> {
    let mut lines = StreamLines::default();
    let mut content = String::new();
    while let Some(chunk) = next_chunk()? {
        let events = lines.push(chunk.as_ref())?;
        if write_events(terminal, events, &mut content)? {
            return Ok(content);
        }
    }
    if let Some(event) = lines.finish()? {
        write_events(terminal, vec![event], &mut content)?;
    }
    Ok(content)
}

pub(super) fn write_events(
    terminal: &mut dyn Terminal,
    events: Vec<LineEvent>,
    content: &mut String,
) -> Result<bool, ChatError> {
    for event in events {
        match event {
            LineEvent::Ignored => {}
            LineEvent::Content(fragment) => {
                terminal.write_fragment(&fragment)?;
                content.push_str(&fragment);
                if content.len() > BODY_LIMIT_BYTES {
                    return Err(ChatError::ReplyTooLarge {
                        limit: BODY_LIMIT_BYTES,
                    });
                }
            }
            LineEvent::Done => return Ok(true),
        }
    }
    Ok(false)
}

fn report_http_error(
    terminal: &mut dyn Terminal,
    status: u16,
    read: Result<Vec<u8>, ReadError>,
) -> Result<String, ChatError> {
    let body = match read {
        Ok(body) => body,
        Err(ReadError::TooLarge { limit }) => return Err(ChatError::BodyTooLarge { limit }),
        Err(ReadError::Failed(source)) => return Err(ChatError::Http(HttpError::Request(source))),
    };
    terminal.write_error_line(&format!("Error: HTTP {status}"))?;
    let detail = match serde_json::from_slice::<StreamErrorBody>(&body) {
        Ok(view) => view
            .error
            .and_then(|found| found.message)
            .unwrap_or_else(|| String::from("Unknown error")),
        Err(_shape) => match serde_json::from_slice::<&RawValue>(&body) {
            Ok(_json) => String::from("Unknown error"),
            Err(_not_json) => match from_utf8(&body) {
                Ok(text) => text.to_owned(),
                Err(_invalid) => ChatError::NotUtf8.to_string(),
            },
        },
    };
    terminal.write_error_line(&detail)?;
    Ok(String::new())
}

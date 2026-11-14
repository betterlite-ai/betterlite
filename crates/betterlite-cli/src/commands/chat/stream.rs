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

#[expect(
    clippy::too_many_lines,
    reason = "the streamed response type cannot be named outside betterlite-http, so opening it, reading an error body and the chunk loop share one function"
)]
pub(super) fn stream_reply(
    client: &Client,
    terminal: &mut dyn Terminal,
    request: StreamRequest<'_>,
) -> Result<String, ChatError> {
    let body = serde_json::to_vec(&CompletionsBody {
        model: request.model,
        messages: request.messages,
        stream: true,
        temperature: request.temperature,
        max_tokens: request.max_tokens,
    })
    .map_err(ChatError::Encode)?;
    let headers = build_headers(
        vec![(String::from(CONTENT_TYPE.0), String::from(CONTENT_TYPE.1))],
        client.api_key.as_ref(),
    )
    .map_err(ChatError::Client)?;
    let url = client
        .resolve(CHAT_COMPLETIONS_PATH)
        .map_err(ChatError::Client)?;
    let built = client
        .http
        .client
        .post(url)
        .headers(headers)
        .body(body)
        .build()
        .map_err(|source| ChatError::Http(HttpError::Request(source)))?;
    let _entered = client.runtime.enter();
    let mut response = match client
        .runtime
        .block_on(timeout(STREAM_TIMEOUT, client.http.execute(built)))
    {
        Ok(Ok(response)) => response,
        Ok(Err(source)) => return report(terminal, ChatError::Http(source)),
        Err(_elapsed) => {
            return report(
                terminal,
                ChatError::Timeout {
                    seconds: STREAM_TIMEOUT.as_secs(),
                },
            );
        }
    };
    let status = response.status().as_u16();
    if status == 401 {
        return report(terminal, ChatError::Status { status });
    }
    if status >= 400 {
        let read = client.runtime.block_on(timeout(
            STREAM_TIMEOUT,
            read_bounded(&mut response, BODY_LIMIT_BYTES),
        ));
        let error_body = match read {
            Ok(Ok(error_body)) => error_body,
            Ok(Err(ReadError::TooLarge { limit })) => {
                return report(terminal, ChatError::BodyTooLarge { limit });
            }
            Ok(Err(ReadError::Failed(source))) => {
                return report(terminal, ChatError::Http(HttpError::Request(source)));
            }
            Err(_elapsed) => {
                return report(
                    terminal,
                    ChatError::Timeout {
                        seconds: STREAM_TIMEOUT.as_secs(),
                    },
                );
            }
        };
        return report_http_error(terminal, status, &error_body);
    }
    let header_limit = client.http.limits.header_bytes;
    if check_headers(response.headers(), header_limit).is_err() {
        return report(
            terminal,
            ChatError::Headers {
                limit: header_limit,
            },
        );
    }
    let mut lines = StreamLines::default();
    let mut content = String::new();
    loop {
        match client
            .runtime
            .block_on(timeout(STREAM_TIMEOUT, response.chunk()))
        {
            Ok(Ok(Some(chunk))) => {
                let events = match lines.push(&chunk) {
                    Ok(events) => events,
                    Err(source) => return report(terminal, ChatError::Line(source)),
                };
                match write_events(terminal, events, &mut content) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) => return report(terminal, error),
                }
            }
            Ok(Ok(None)) => {
                match lines.finish() {
                    Ok(Some(event)) => {
                        if let Err(error) = write_events(terminal, vec![event], &mut content) {
                            return report(terminal, error);
                        }
                    }
                    Ok(None) => {}
                    Err(source) => return report(terminal, ChatError::Line(source)),
                }
                break;
            }
            Ok(Err(source)) => {
                return report(terminal, ChatError::Http(HttpError::Request(source)));
            }
            Err(_elapsed) => {
                return report(
                    terminal,
                    ChatError::Timeout {
                        seconds: STREAM_TIMEOUT.as_secs(),
                    },
                );
            }
        }
    }
    terminal.write_line("")?;
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

fn report(terminal: &mut dyn Terminal, error: ChatError) -> Result<String, ChatError> {
    terminal.write_error_line(&format!("Error: {error}"))?;
    Ok(String::new())
}

fn report_http_error(
    terminal: &mut dyn Terminal,
    status: u16,
    body: &[u8],
) -> Result<String, ChatError> {
    terminal.write_error_line(&format!("Error: HTTP {status}"))?;
    let detail = match serde_json::from_slice::<StreamErrorBody>(body) {
        Ok(view) => view
            .error
            .and_then(|found| found.message)
            .unwrap_or_else(|| String::from("Unknown error")),
        Err(_shape) => match serde_json::from_slice::<&RawValue>(body) {
            Ok(_json) => String::from("Unknown error"),
            Err(_not_json) => match std::str::from_utf8(body) {
                Ok(text) => text.to_owned(),
                Err(_invalid) => ChatError::NotUtf8.to_string(),
            },
        },
    };
    terminal.write_error_line(&detail)?;
    Ok(String::new())
}

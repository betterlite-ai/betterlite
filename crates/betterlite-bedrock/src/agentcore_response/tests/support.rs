use std::future::{self, Future};
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_openai::chat::response::{ChatResponse, ChatUsage};
use betterlite_tokenizer::count::request::{Message, TokenRequest};
use tracing::subscriber::{DefaultGuard, set_default};

use crate::agentcore_response::counter::TokenCounter;
use crate::agentcore_response::transform::RawResponse;

pub(super) fn response() -> ChatResponse {
    ChatResponse {
        id: "chatcmpl-fixture".to_owned(),
        object: "chat.completion".to_owned(),
        created: 42,
        model: "supplied-model".to_owned(),
        choices: Vec::new(),
        usage: ChatUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        system_fingerprint: Some("fp-fixture".to_owned()),
        service_tier: Some("standard".to_owned()),
    }
}

pub(super) fn sse_response(body: &str) -> RawResponse {
    raw_response(200, "text/event-stream", body)
}

pub(super) fn json_response(body: &str) -> RawResponse {
    raw_response(200, "application/json", body)
}

pub(super) fn raw_response(status_code: u16, content_type: &str, body: &str) -> RawResponse {
    RawResponse {
        status_code,
        headers: vec![("content-type".to_owned(), content_type.to_owned())],
        body: body.as_bytes().to_vec(),
    }
}

pub(super) struct FixtureCounter<'expected> {
    pub(super) model: &'expected str,
    pub(super) messages: &'expected [Message],
    pub(super) content: &'expected str,
    pub(super) prompt: Option<usize>,
    pub(super) completion: Option<usize>,
}

impl<'expected> FixtureCounter<'expected> {
    pub(super) const fn new(
        model: &'expected str,
        messages: &'expected [Message],
        content: &'expected str,
        prompt: Option<usize>,
        completion: Option<usize>,
    ) -> Self {
        Self {
            model,
            messages,
            content,
            prompt,
            completion,
        }
    }
}

impl TokenCounter for FixtureCounter<'_> {
    type Error = io::Error;

    fn count(&self, request: &TokenRequest<'_>) -> impl Future<Output = Result<usize, io::Error>> {
        if request.model != self.model {
            return future::ready(Err(io::Error::other("fixture counter model mismatch")));
        }
        let outcome = match (
            request.messages,
            request.text,
            request.count_response_tokens,
        ) {
            (Some(messages), None, Some(false)) if same_messages(messages, self.messages) => {
                self.prompt
            }
            (None, Some(text), Some(true)) if text.joined().as_ref() == self.content => {
                self.completion
            }
            _ => {
                return future::ready(Err(io::Error::other(
                    "fixture counter called with an unexpected request",
                )));
            }
        };
        future::ready(outcome.ok_or_else(|| io::Error::other("fixture counter unavailable")))
    }
}

fn same_messages(found: &[Message], expected: &[Message]) -> bool {
    let found_text = format!("{found:?}");
    let expected_text = format!("{expected:?}");
    found_text == expected_text
}

pub(super) fn messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap()
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn choice_message(
    response: &ChatResponse,
) -> (u32, Option<String>, String, Option<String>) {
    let choice = response.choices.first().expect("one choice");
    (
        choice.index,
        choice.finish_reason.clone(),
        choice.message.role.clone(),
        choice.message.content.clone(),
    )
}

#![cfg(test)]

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use tracing::Level;
use tracing::subscriber::set_default;

use super::shared::{analyzer, anonymizer, context, request_data};
use crate::presidio_response::Mode;
use crate::presidio_response::anthropic::process_anthropic_response;
use crate::presidio_response::wire::{AnthropicMessage, ContentItem};

fn message(json: &str) -> AnthropicMessage {
    serde_json::from_str(json).expect("Anthropic message")
}

fn block_texts(message: &AnthropicMessage) -> Vec<Option<String>> {
    message
        .content
        .iter()
        .flatten()
        .map(|item| match item {
            ContentItem::Object(part) => part.text.clone(),
            ContentItem::Other(_) => None,
        })
        .collect()
}

#[tokio::test]
async fn masks_only_text_blocks() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = message(
        r#"{"type":"message","content":[{"type":"text","text":"Alice"},{"type":"tool_use","id":"call-1","name":"lookup"},{"type":"text","text":"Bobby"}]}"#,
    );

    process_anthropic_response(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("anthropic processing");

    assert_eq!(
        block_texts(&response),
        vec![Some("MASKED".to_owned()), None, Some("MASKED".to_owned())]
    );
    let Some(ContentItem::Object(tool_use)) =
        response.content.as_ref().and_then(|items| items.get(1))
    else {
        panic!("the tool_use block survives");
    };
    assert_eq!(tool_use.kind.as_deref(), Some("tool_use"));
    assert_eq!(
        tool_use.extra.get("id").map(|value| value.get()),
        Some(r#""call-1""#)
    );
}

#[tokio::test]
async fn skips_blocks_that_are_not_text() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response =
        message(r#"{"type":"message","content":["bare",{"type":"text"},{"type":"image"}]}"#);

    process_anthropic_response(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("anthropic processing");

    assert_eq!(block_texts(&response), vec![None, None, None]);
    let Some(ContentItem::Other(bare)) = response.content.as_ref().and_then(|items| items.first())
    else {
        panic!("the bare block stays a raw value");
    };
    assert_eq!(bare.get(), r#""bare""#);
}

#[tokio::test]
async fn unmasks_text_blocks_from_request_metadata() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data(r#"{"metadata":{"pii_tokens":{"<PERSON_1>":"Alice"}}}"#);
    let mut response =
        message(r#"{"type":"message","content":[{"type":"text","text":"Hi <PERSON_1>"}]}"#);

    process_anthropic_response(context, &mut response, &mut data, Mode::Unmask)
        .await
        .expect("anthropic processing");

    assert_eq!(block_texts(&response), vec![Some("Hi Alice".to_owned())]);
}

#[tokio::test]
async fn a_non_list_content_is_left_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = message(r#"{"type":"message","content":"Alice"}"#);

    process_anthropic_response(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("anthropic processing");

    let raw = serde_json::to_string(&response).expect("message JSON");
    assert_eq!(raw, r#"{"content":"Alice","type":"message"}"#);
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn unmasking_without_tokens_logs_a_debug_line() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Sink(Arc::clone(&captured));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_writer(move || sink.clone())
        .finish();
    let _guard = set_default(subscriber);
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response =
        message(r#"{"type":"message","content":[{"type":"text","text":"Hi <PERSON_1>"}]}"#);

    process_anthropic_response(context, &mut response, &mut data, Mode::Unmask)
        .await
        .expect("anthropic processing");

    let text = String::from_utf8(captured.lock().expect("captured log").clone())
        .expect("captured log is UTF-8");
    assert!(text.contains("pii_tokens"));
}

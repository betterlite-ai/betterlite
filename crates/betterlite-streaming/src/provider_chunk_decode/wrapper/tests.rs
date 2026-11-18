#![cfg(test)]

use rstest::rstest;
use uuid::Uuid;

use crate::provider_chunk_decode::chunk::ProviderChunk;
use crate::provider_chunk_decode::wrapper::{StreamSetup, StreamWrapper};
use crate::stream_chunk_shaping::creation::StreamInstant;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk, StreamOptions};

const CONTENT: &str = r#"{"choices":[{"index":0,"text":"hi"}],"usage":null}"#;
const COUNTED_CONTENT: &str = r#"{"choices":[{"index":0,"text":"hi"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
const USAGE_ONLY: &str =
    r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.5,
    }
}

fn fixed_id() -> Uuid {
    Uuid::nil()
}

fn setup(stream_options: Option<StreamOptions>) -> StreamSetup {
    StreamSetup {
        model: "gpt-4".to_owned(),
        stream_options,
        merge_reasoning_in_choices: false,
        is_function_call: false,
        base_hidden_params: JsonObject::new(),
        repeated_chunk_limit: 100,
        now: fixed_instant,
        next_id: fixed_id,
    }
}

fn chunk(source: &str) -> ProviderChunk {
    let value: JsonValue = serde_json::from_str(source).expect("the test literal is valid JSON");
    ProviderChunk::Json(Box::new(value))
}

fn closing_chunk(wrapper: &mut StreamWrapper) -> StreamChunk {
    wrapper
        .finish()
        .expect("the wrapper sends the closing chunk")
        .expect("the closing chunk shapes")
}

fn closing_finish_reason() {
    let mut wrapper = StreamWrapper::new("text-completion-openai", setup(None));
    let content = wrapper
        .push(&chunk(CONTENT))
        .expect("the content chunk shapes")
        .expect("the content chunk goes out");
    assert!(!content.hidden_params.contains_key("usage"));
    let closing = closing_chunk(&mut wrapper);
    let finish_reason = closing
        .choices
        .first()
        .and_then(|choice| choice.finish_reason.as_deref());
    assert_eq!(finish_reason, Some("stop"));
    assert!(
        closing.hidden_params.contains_key("usage"),
        "the closing chunk carries the running total"
    );
    assert!(
        wrapper.finish().is_none(),
        "the closing chunk goes out once"
    );
}

fn usage_chunk_when_asked() {
    let options = StreamOptions {
        include_usage: true,
    };
    let mut wrapper = StreamWrapper::new("text-completion-openai", setup(Some(options)));
    assert!(
        wrapper
            .push(&chunk(USAGE_ONLY))
            .expect("the usage chunk is recorded silently")
            .is_none()
    );
    assert!(
        wrapper
            .push(&chunk(CONTENT))
            .expect("the content chunk shapes")
            .is_some()
    );
    let closing = closing_chunk(&mut wrapper);
    assert!(closing.usage.is_none());
    assert!(!closing.hidden_params.contains_key("usage"));
    let usage = wrapper
        .finish()
        .expect("the wrapper sends the usage chunk after the closing chunk")
        .expect("the usage chunk shapes")
        .usage
        .expect("the usage chunk carries the totals");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 5);
    assert!(wrapper.finish().is_none(), "the usage chunk goes out once");
    let late = wrapper
        .push(&chunk(COUNTED_CONTENT))
        .expect("the late chunk shapes")
        .expect("the late chunk goes out");
    assert!(!late.hidden_params.contains_key("usage"));
}

fn late_chunk_keeps_totals() {
    let mut wrapper = StreamWrapper::new("text-completion-openai", setup(None));
    wrapper
        .push(&chunk(CONTENT))
        .expect("the content chunk shapes")
        .expect("the content chunk goes out");
    closing_chunk(&mut wrapper);
    let late = wrapper
        .push(&chunk(CONTENT))
        .expect("the late chunk shapes")
        .expect("the late chunk goes out");
    assert!(
        late.hidden_params.contains_key("usage"),
        "a chunk after the closing chunk keeps the running totals"
    );
}

fn silent_usage_chunk_dropped() {
    let options = StreamOptions {
        include_usage: true,
    };
    let mut wrapper = StreamWrapper::new("openai", setup(Some(options)));
    let pushed = wrapper
        .push(&chunk(USAGE_ONLY))
        .expect("the usage chunk decodes");
    assert!(pushed.is_none(), "a usage-only chunk is never sent");
}

#[rstest]
#[case::closing_finish_reason(closing_finish_reason)]
#[case::usage_chunk_when_asked(usage_chunk_when_asked)]
#[case::late_chunk_keeps_totals(late_chunk_keeps_totals)]
#[case::silent_usage_chunk_dropped(silent_usage_chunk_dropped)]
fn stream_wrapper_cases(#[case] scenario: fn()) {
    scenario();
}

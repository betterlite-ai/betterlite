#![cfg(test)]

use std::time::Duration;

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use crate::gemini_call::selection::{CallSelection, pop_stream, select, sync_client_timeout};
use crate::gemini_http::error::GeminiHttpError;
use crate::vertex_embedding_call::post::{ClientTimeout, Timeout};

fn params(source: &str) -> Object {
    serde_json::from_str(source).expect("fixture params")
}

#[rstest]
#[case::stream_absent(
    false,
    None,
    CallSelection {
        asynchronous: false,
        streaming: false
    }
)]
#[case::stream_false(
    false,
    Some(false),
    CallSelection {
        asynchronous: false,
        streaming: false
    }
)]
#[case::stream_true(
    false,
    Some(true),
    CallSelection {
        asynchronous: false,
        streaming: true
    }
)]
#[case::async_complete(
    true,
    None,
    CallSelection {
        asynchronous: true,
        streaming: false
    }
)]
#[case::async_stream_false(
    true,
    Some(false),
    CallSelection {
        asynchronous: true,
        streaming: false
    }
)]
#[case::async_stream(
    true,
    Some(true),
    CallSelection {
        asynchronous: true,
        streaming: true
    }
)]
fn selection_modes(
    #[case] acompletion: bool,
    #[case] stream: Option<bool>,
    #[case] expected: CallSelection,
) {
    assert_eq!(select(acompletion, stream), expected);
}

#[test]
fn selection_pops_a_true_stream_flag() {
    let mut optional_params = params(r#"{"stream":true,"temperature":0.2}"#);
    let stream = pop_stream(&mut optional_params);
    assert_eq!(stream.ok(), Some(Some(true)));
    assert!(!optional_params.contains_key("stream"));
    assert!(optional_params.contains_key("temperature"));
}

#[test]
fn selection_pops_a_false_stream_flag() {
    let mut optional_params = params(r#"{"stream":false}"#);
    let stream = pop_stream(&mut optional_params);
    assert_eq!(stream.ok(), Some(Some(false)));
    assert!(!optional_params.contains_key("stream"));
}

#[test]
fn selection_treats_a_null_stream_flag_as_absent() {
    let mut optional_params = params(r#"{"stream":null}"#);
    let stream = pop_stream(&mut optional_params);
    assert_eq!(stream.ok(), Some(None));
    assert!(!optional_params.contains_key("stream"));
}

#[test]
fn selection_treats_a_missing_stream_flag_as_absent() {
    let mut optional_params = params(r#"{"temperature":0.2}"#);
    let stream = pop_stream(&mut optional_params);
    assert_eq!(stream.ok(), Some(None));
    assert!(!optional_params.contains_key("stream"));
}

#[test]
fn selection_rejects_a_stream_flag_that_is_not_boolean() {
    let mut optional_params = params(r#"{"stream":1}"#);
    let error = pop_stream(&mut optional_params);
    let Err(GeminiHttpError::Flag { field }) = error else {
        panic!("a number is not the typed boolean option");
    };
    assert_eq!(field, "stream");
    assert!(!optional_params.contains_key("stream"));
}

#[rstest]
#[case::explicit_zero_timeout(
    Some(Duration::ZERO),
    ClientTimeout::Explicit(Timeout {
        total: Duration::ZERO,
        connect: Duration::ZERO
    })
)]
#[case::explicit_timeout(
    Some(Duration::from_secs(30)),
    ClientTimeout::Explicit(Timeout {
        total: Duration::from_secs(30),
        connect: Duration::from_secs(30)
    })
)]
#[case::absent_timeout(None, ClientTimeout::HandlerDefault)]
fn selection_sync_client_timeout(
    #[case] timeout: Option<Duration>,
    #[case] expected: ClientTimeout,
) {
    assert_eq!(sync_client_timeout(timeout), expected);
}

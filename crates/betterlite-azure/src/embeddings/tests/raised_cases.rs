use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;

use crate::embeddings::error::EmbeddingError;
use crate::embeddings::logging::ResponseKind;

use super::error_cases::azure;
use super::support::{
    Answer, BODY, Calls, Recorder, Run, call, headers, run, unresolved_client_run,
};

// a full 200 embeddings body that also carries an "error" field, as an OpenAI-compatible provider sends it
fn with_error(error: &str) -> Answer {
    let open = BODY.strip_suffix('}').unwrap();
    Answer::Reply {
        status: 200,
        headers: headers(&[("content-type", "application/json")]),
        body: format!("{open},\"error\":{error}}}"),
    }
}

fn sent(error: &str, aembedding: bool) -> Run {
    let input = EmbeddingInput::Text("a".to_owned());
    run(
        call(&input, aembedding),
        &with_error(error),
        &Calls::default(),
        Recorder::default(),
    )
}

// the sync path wraps the bare exception, whose str() is empty, and the async path raises it with its attributes
fn assert_raised(error: &str, status: u16, message: &str) {
    let sync = azure(sent(error, false));
    assert_eq!((sync.status_code, sync.message.as_str()), (status, ""));
    let outcome = sent(error, true);
    assert_eq!(outcome.recorder.post.len(), 2);
    assert_eq!(outcome.recorder.post[0].kind, ResponseKind::Dumped);
    assert_eq!(outcome.recorder.post[1].kind, ResponseKind::ErrorText);
    assert_eq!(
        outcome.recorder.post[1].original_response,
        Json::Text(String::new())
    );
    let Err(EmbeddingError::Response {
        status_code,
        message: raised,
    }) = outcome.result
    else {
        panic!("expected the unwrapped response error for {error}");
    };
    assert_eq!((status_code, raised.as_str()), (status, message));
}

fn assert_converted(error: &str) {
    for aembedding in [false, true] {
        let outcome = sent(error, aembedding);
        assert!(outcome.result.unwrap().is_some(), "{error}");
    }
}

pub(super) fn answer_errors() {
    assert_raised(r#"{"message":"boom","code":400}"#, 400, "boom");
    assert_raised(r#"{"message":"boom"}"#, 422, "boom");
    assert_raised(r#"{"code":429}"#, 429, "Error in response object");
    assert_raised(r#"{"message":"","code":503}"#, 503, "");
    assert_raised(r#"{"message":{"a":1}}"#, 422, r#"{"a":1}"#);
    assert_raised(r#""bad""#, 422, "Error in response object");
    assert_raised("7", 422, "Error in response object");
    assert_raised("[]", 422, "Error in response object");
    assert_raised(r#"{"message":"m","code":"content_filter"}"#, 500, "m");
    assert_raised(r#"{"message":"m","code":70000}"#, 500, "m");
    assert_converted("null");
    assert_converted(r#""""#);
    assert_converted("{}");
    assert_converted(r#"{"message":"","code":null}"#);
    assert_converted(r#"{"message":null}"#);
}

// a failing client resolution keeps the status the source error carries
pub(super) fn unresolved_client() {
    let sync = azure(unresolved_client_run(false, Some(401)));
    assert_eq!(sync.status_code, 401);
    assert_eq!(sync.message, "no credential source in this fixture");
    assert_eq!(azure(unresolved_client_run(false, None)).status_code, 500);
    let outcome = unresolved_client_run(true, Some(401));
    assert!(outcome.calls.is_empty());
    assert_eq!(outcome.recorder.post.len(), 1);
    assert_eq!(outcome.recorder.post[0].kind, ResponseKind::ErrorText);
    assert_eq!(
        outcome.recorder.post[0].original_response,
        Json::Text("no credential source in this fixture".to_owned())
    );
    let Err(EmbeddingError::Endpoint(raised)) = outcome.result else {
        panic!("expected the unwrapped endpoint error");
    };
    assert_eq!(raised.status(), Some(401));
}

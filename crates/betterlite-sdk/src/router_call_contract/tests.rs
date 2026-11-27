#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use betterlite_core::api::passthrough_call::{
    PassthroughBody, PassthroughCallError, PassthroughReply,
};
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_models::decisions::{HiddenParams, Json};
use bytes::Bytes;

use crate::router_call_contract::error::{FailureCause, GenericError, GenericFailure};
use crate::router_call_contract::{GenericOutcome, GenericStream};

struct ScriptedStream {
    chunks: Vec<Vec<u8>>,
    position: usize,
    closed: Arc<AtomicBool>,
    hidden: HiddenParams,
    buffered: bool,
    chunk_dumps: Vec<Json>,
    messages: Vec<Json>,
    model: Option<String>,
}

#[async_trait]
impl GenericStream for ScriptedStream {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, GenericError> {
        let chunk = self.chunks.get(self.position).cloned();
        if chunk.is_some() {
            self.position = self.position.saturating_add(1);
        }
        Ok(chunk)
    }

    async fn close(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn hidden_params(&self) -> HiddenParams {
        self.hidden.clone()
    }

    fn has_buffered_provider_output(&self) -> bool {
        self.buffered
    }

    fn chunks(&self) -> Option<&[Json]> {
        Some(self.chunk_dumps.as_slice())
    }

    fn messages(&self) -> Option<&[Json]> {
        Some(self.messages.as_slice())
    }

    fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }
}

fn scripted_stream(closed: Arc<AtomicBool>) -> ScriptedStream {
    ScriptedStream {
        chunks: vec![b"first".to_vec(), b"second".to_vec()],
        position: 0,
        closed,
        hidden: HiddenParams::from([(
            String::from("model_id"),
            Json::Text(String::from("deployment-1")),
        )]),
        buffered: true,
        chunk_dumps: vec![Json::Text(String::from("first dump"))],
        messages: vec![Json::Text(String::from("hello"))],
        model: Some(String::from("gpt-4o")),
    }
}

fn unclassified(message: &str) -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message: String::from(message),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

#[test]
fn outcome_http_hands_back_the_passthrough_reply() {
    let outcome = GenericOutcome::Http(PassthroughReply {
        status: 206,
        headers: vec![(String::from("x-relay"), String::from("yes"))],
        body: PassthroughBody::Full(Box::new(Bytes::from_static(b"partial body"))),
    });

    let GenericOutcome::Http(reply) = outcome else {
        panic!("an http outcome stays the passthrough reply");
    };
    assert_eq!(reply.status, 206);
    assert_eq!(
        reply.headers,
        vec![(String::from("x-relay"), String::from("yes"))]
    );
    let PassthroughBody::Full(body) = reply.body else {
        panic!("the reply carries its full body");
    };
    assert_eq!(&body[..], b"partial body");
}

#[test]
fn outcome_bytes_hands_back_its_bytes() {
    let outcome = GenericOutcome::Bytes(b"container content".to_vec());

    let GenericOutcome::Bytes(bytes) = outcome else {
        panic!("a byte outcome stays bytes");
    };
    assert_eq!(bytes, b"container content".to_vec());
}

#[tokio::test]
async fn outcome_stream_yields_its_chunks_and_then_ends() {
    let closed = Arc::new(AtomicBool::new(false));

    let outcome = GenericOutcome::Stream(Box::new(scripted_stream(Arc::clone(&closed))));

    let GenericOutcome::Stream(mut stream) = outcome else {
        panic!("a stream outcome stays a stream");
    };
    assert_eq!(
        stream.next_chunk().await.expect("the first chunk"),
        Some(b"first".to_vec())
    );
    assert_eq!(
        stream.next_chunk().await.expect("the second chunk"),
        Some(b"second".to_vec())
    );
    assert_eq!(stream.next_chunk().await.expect("the stream ends"), None);
    assert_eq!(
        stream.next_chunk().await.expect("the end stays the end"),
        None
    );
    stream.close().await;
    assert!(closed.load(Ordering::SeqCst));
}

#[test]
fn outcome_stream_answers_its_readers() {
    let closed = Arc::new(AtomicBool::new(false));
    let chunk_dumps = vec![Json::Text(String::from("first dump"))];
    let messages = vec![Json::Text(String::from("hello"))];
    let hidden = HiddenParams::from([(
        String::from("model_id"),
        Json::Text(String::from("deployment-1")),
    )]);

    let outcome = GenericOutcome::Stream(Box::new(scripted_stream(closed)));

    let GenericOutcome::Stream(stream) = outcome else {
        panic!("a stream outcome stays a stream");
    };
    assert_eq!(stream.hidden_params(), hidden);
    assert!(stream.has_buffered_provider_output());
    assert_eq!(stream.chunks(), Some(chunk_dumps.as_slice()));
    assert_eq!(stream.messages(), Some(messages.as_slice()));
    assert_eq!(stream.model(), Some("gpt-4o"));
}

#[test]
fn error_unknown_call_type_names_the_call_type() {
    let error = GenericError::UnknownCallType(String::from("acreate_batch"));

    assert_eq!(
        error.to_string(),
        "no executor is registered for the call type acreate_batch"
    );
}

#[test]
fn error_class_cause_shows_the_exception_text_and_keeps_the_stamps() {
    let class = unclassified("the provider refused the call");
    let message = class_message(&class);
    let error = GenericError::Failed(Box::new(GenericFailure {
        cause: FailureCause::Class(class),
        failed_deployment_id: Some(String::from("deployment-1")),
        retry_skip_deployment_id: Some(String::from("deployment-2")),
    }));

    let GenericError::Failed(failure) = error else {
        panic!("a class failure stays failed");
    };
    assert_eq!(
        failure.failed_deployment_id.as_deref(),
        Some("deployment-1")
    );
    assert_eq!(
        failure.retry_skip_deployment_id.as_deref(),
        Some("deployment-2")
    );
    assert_eq!(failure.to_string(), message);
    assert_eq!(failure.to_string(), "the provider refused the call");
}

#[test]
fn error_argument_cause_names_the_key_and_expected_type() {
    let error = GenericError::Failed(Box::new(GenericFailure {
        cause: FailureCause::Argument {
            key: String::from("input"),
            expected: "a list of messages",
        },
        failed_deployment_id: None,
        retry_skip_deployment_id: None,
    }));

    assert_eq!(
        error.to_string(),
        "the input argument is not a list of messages"
    );
}

#[test]
fn error_passthrough_cause_shows_its_own_text() {
    let error = GenericError::from(PassthroughCallError::ProviderNotFound(String::from(
        "openai",
    )));

    assert_eq!(error.to_string(), "Provider openai not found");
}

#[test]
fn error_from_error_class_fills_no_deployment_ids() {
    let error = GenericError::from(unclassified("the provider refused"));

    let GenericError::Failed(failure) = error else {
        panic!("a class becomes a failed call");
    };
    assert_eq!(failure.failed_deployment_id, None);
    assert_eq!(failure.retry_skip_deployment_id, None);
    assert!(matches!(failure.cause, FailureCause::Class(_)));
}

#[test]
fn error_from_passthrough_fills_no_deployment_ids() {
    let error = GenericError::from(PassthroughCallError::ProviderNotFound(String::from(
        "openai",
    )));

    let GenericError::Failed(failure) = error else {
        panic!("a passthrough error becomes a failed call");
    };
    assert_eq!(failure.failed_deployment_id, None);
    assert_eq!(failure.retry_skip_deployment_id, None);
    assert!(matches!(failure.cause, FailureCause::Passthrough(_)));
}

#[test]
fn error_passthrough_upstream_status_comes_back_unchanged() {
    let error = GenericError::from(PassthroughCallError::UpstreamStatus {
        status: 429,
        headers: vec![(String::from("retry-after"), String::from("5"))],
        body: Bytes::from_static(b"rate limited"),
    });

    let GenericError::Failed(failure) = error else {
        panic!("an upstream status becomes a failed call");
    };
    let GenericFailure { cause, .. } = *failure;
    let FailureCause::Passthrough(PassthroughCallError::UpstreamStatus {
        status,
        headers,
        body,
    }) = cause
    else {
        panic!("the passthrough error stays whole");
    };
    assert_eq!(status, 429);
    assert_eq!(
        headers,
        vec![(String::from("retry-after"), String::from("5"))]
    );
    assert_eq!(&body[..], b"rate limited");
}

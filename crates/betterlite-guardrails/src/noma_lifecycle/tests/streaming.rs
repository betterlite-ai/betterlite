use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use rstest::rstest;
use serde_json::json;
use thiserror::Error;
use tokio::sync::mpsc::channel;

use super::support::{
    ANONYMIZING_ASSISTANT_REPLY, BLOCKING_REPLY, SAFE_REPLY, USER_MESSAGES, build, data, field,
    hooked, mock_noma, params, records, request_count, scanned_text,
};
use crate::cato::error::ModelStreamError;
use crate::noma_lifecycle::error::NomaLifecycleError;
use crate::noma_lifecycle::stream_error::NomaStreamError;
use crate::noma_lifecycle::streaming::{AssembledStream, StreamAssembly, StreamHookRequest};
use crate::noma_lifecycle::{NomaGuardrail, RequestData};
use crate::noma_scan_policy::error::NomaError;

#[derive(Debug, Error)]
#[error("the fake assembly failed")]
struct FakeFailure;

#[derive(Clone, Copy)]
enum Assembly {
    Model,
    Other,
    FailToAssemble,
    FailToReplay,
}

fn model_response(content: &str) -> GuardrailValue {
    let response = json!({
        "id": "chatcmpl-1",
        "choices": [{"message": {"role": "assistant", "content": content}}],
    });
    serde_json::from_value(response).unwrap()
}

// the chunks are plain text pieces that join into the content, and the replay wraps the response in a list
impl StreamAssembly for Assembly {
    type Error = FakeFailure;

    fn assemble(&self, chunks: &[GuardrailValue]) -> Result<AssembledStream, FakeFailure> {
        match self {
            Self::Model => {
                let content: String = chunks
                    .iter()
                    .filter_map(|chunk| {
                        if let GuardrailValue::Text(piece) = chunk {
                            Some(piece.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                Ok(AssembledStream::Model(model_response(&content)))
            }
            Self::Other => Ok(AssembledStream::Other),
            Self::FailToAssemble => Err(FakeFailure),
            Self::FailToReplay => Ok(AssembledStream::Model(model_response("hello"))),
        }
    }

    fn replay(&self, response: GuardrailValue) -> Result<GuardrailValue, FakeFailure> {
        match self {
            Self::FailToReplay => Err(FakeFailure),
            Self::Model | Self::Other | Self::FailToAssemble => {
                Ok(GuardrailValue::Array(vec![response]))
            }
        }
    }
}

fn pieces(texts: &[&str]) -> Vec<Result<GuardrailValue, ModelStreamError>> {
    texts
        .iter()
        .map(|piece| Ok(GuardrailValue::Text((*piece).to_owned())))
        .collect()
}

fn original(texts: &[&str]) -> Vec<GuardrailValue> {
    texts
        .iter()
        .map(|piece| GuardrailValue::Text((*piece).to_owned()))
        .collect()
}

async fn stream(
    guardrail: &NomaGuardrail,
    request_data: &mut RequestData,
    items: Vec<Result<GuardrailValue, ModelStreamError>>,
    assembly: Assembly,
) -> Result<Vec<GuardrailValue>, NomaStreamError<FakeFailure>> {
    let (sender, receiver) = channel(items.len().max(1));
    for item in items {
        sender.send(item).await.unwrap();
    }
    drop(sender);
    let request = StreamHookRequest {
        user_auth: &BTreeMap::new(),
        data: request_data,
    };
    guardrail
        .async_post_call_streaming_iterator_hook(request, receiver, &assembly)
        .await
}

#[tokio::test]
async fn the_stream_is_buffered_assembled_scanned_and_replayed() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let yielded = stream(
        &guardrail,
        &mut request_data,
        pieces(&["hel", "lo"]),
        Assembly::Model,
    )
    .await
    .unwrap();
    assert_eq!(
        yielded,
        vec![GuardrailValue::Array(vec![model_response("hello")])]
    );
    assert_eq!(request_count(&mock), 1);
    assert_eq!(scanned_text(&mock, 0), "hello");
    let written = records(&request_data);
    assert_eq!(
        field(&written[0], "guardrail_mode").cloned(),
        Some(GuardrailValue::Text(String::from("post_call")))
    );
}

#[tokio::test]
async fn an_empty_stream_yields_nothing_and_scans_nothing() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let yielded = stream(&guardrail, &mut request_data, Vec::new(), Assembly::Model)
        .await
        .unwrap();
    assert!(yielded.is_empty());
    assert_eq!(request_count(&mock), 0);
}

#[tokio::test]
async fn a_response_that_is_not_a_model_response_passes_the_original_chunks() {
    let mock = mock_noma(StatusCode::OK, BLOCKING_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let yielded = stream(
        &guardrail,
        &mut request_data,
        pieces(&["a", "b"]),
        Assembly::Other,
    )
    .await
    .unwrap();
    assert_eq!(yielded, original(&["a", "b"]));
    assert_eq!(request_count(&mock), 0);
}

#[tokio::test]
async fn an_anonymized_response_is_what_gets_replayed() {
    let mock = mock_noma(StatusCode::OK, ANONYMIZING_ASSISTANT_REPLY).await;
    let mut configured = params(&mock.base);
    configured.anonymize_input = Some(true);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    let yielded = stream(
        &guardrail,
        &mut request_data,
        pieces(&["hel", "lo"]),
        Assembly::Model,
    )
    .await
    .unwrap();
    assert_eq!(
        yielded,
        vec![GuardrailValue::Array(vec![model_response("clean")])]
    );
}

#[rstest]
#[case::failures_block(true)]
#[case::failures_allowed(false)]
#[tokio::test]
async fn a_block_is_raised_whatever_block_failures_says(#[case] block_failures: bool) {
    let mock = mock_noma(StatusCode::OK, BLOCKING_REPLY).await;
    let mut configured = params(&mock.base);
    configured.block_failures = Some(block_failures);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    let error = stream(
        &guardrail,
        &mut request_data,
        pieces(&["hel", "lo"]),
        Assembly::Model,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        NomaStreamError::Lifecycle(NomaLifecycleError::Noma(NomaError::Blocked(_)))
    ));
}

#[tokio::test]
async fn a_failed_scan_raises_when_failures_block() {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let error = stream(
        &guardrail,
        &mut request_data,
        pieces(&["hel", "lo"]),
        Assembly::Model,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        NomaStreamError::Lifecycle(NomaLifecycleError::Noma(NomaError::HttpStatus { .. }))
    ));
}

#[tokio::test]
async fn a_failed_scan_passes_the_original_chunks_when_failures_are_allowed() {
    let mock = mock_noma(StatusCode::INTERNAL_SERVER_ERROR, "{}").await;
    let mut configured = params(&mock.base);
    configured.block_failures = Some(false);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    let yielded = stream(
        &guardrail,
        &mut request_data,
        pieces(&["hel", "lo"]),
        Assembly::Model,
    )
    .await
    .unwrap();
    assert_eq!(yielded, original(&["hel", "lo"]));
    assert_eq!(request_count(&mock), 1);
}

#[tokio::test]
async fn the_stream_scan_ignores_the_configured_event_hook() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&hooked(&mock.base, "pre_call"));
    let mut request_data = data(USER_MESSAGES);
    stream(
        &guardrail,
        &mut request_data,
        pieces(&["hel", "lo"]),
        Assembly::Model,
    )
    .await
    .unwrap();
    assert_eq!(request_count(&mock), 1);
}

#[tokio::test]
async fn a_failed_model_stream_is_raised_before_any_scan() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let guardrail = build(&params(&mock.base));
    let mut request_data = data(USER_MESSAGES);
    let mut items = pieces(&["hel"]);
    items.push(Err(ModelStreamError::Provider {
        message: Box::from("overloaded"),
    }));
    let error = stream(&guardrail, &mut request_data, items, Assembly::Model)
        .await
        .unwrap_err();
    assert!(matches!(error, NomaStreamError::Model { .. }));
    assert_eq!(request_count(&mock), 0);
}

#[rstest]
#[case::assembling(Assembly::FailToAssemble, 0)]
#[case::replaying(Assembly::FailToReplay, 1)]
#[tokio::test]
async fn an_assembly_failure_is_raised(#[case] assembly: Assembly, #[case] scans: usize) {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let mut configured = params(&mock.base);
    configured.block_failures = Some(false);
    let guardrail = build(&configured);
    let mut request_data = data(USER_MESSAGES);
    let error = stream(&guardrail, &mut request_data, pieces(&["hel"]), assembly)
        .await
        .unwrap_err();
    assert!(matches!(error, NomaStreamError::Assembly { .. }));
    assert_eq!(request_count(&mock), scans);
}

use std::time::Duration;

use betterlite_core::dispatch::error::DispatchError;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;

use super::support::{Harness, chat_body, dig, json, prepared_call, request};
use super::{USER, answer_choices, body_text};
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::{class_name, class_status};
use crate::call_wrapper::target::CallTarget as _;
use crate::completion_call::error::CompletionCallError;
use crate::completion_call::request;
use crate::completion_request::call::{
    PreparedCall, PreparedCompletion, PreparedTimeout, PreparedTimeoutPhases,
};

fn streaming_call() -> PreparedCall {
    let mut call = prepared_call("gpt-4o");
    call.stream = true;
    call
}

// ---- target_ ----

#[tokio::test]
async fn target_prepared_timeout_bounds_the_call() {
    let harness = Harness::new();
    let target = harness.target();
    let mut call = prepared_call("gpt-4o");
    call.timeout = Some(PreparedTimeout::Seconds(0.0));
    let error = Box::pin(target.run(PreparedCompletion::Dispatch(call), false, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CompletionCallError::Dispatch(DispatchError::DeadlineExpired { .. })
    ));
}

#[test]
fn target_timeout_forms_map_to_one_deadline() {
    assert_eq!(
        request::timeout_duration(&PreparedTimeout::Seconds(2.5)),
        Some(Duration::from_millis(2_500))
    );
    let phases = PreparedTimeout::Phases(PreparedTimeoutPhases {
        connect: Some(1.0_f64),
        read: Some(2.0_f64),
        write: None,
        pool: Some(0.5_f64),
    });
    assert_eq!(
        request::timeout_duration(&phases),
        Some(Duration::from_millis(3_500))
    );
    let absent = PreparedTimeout::Phases(PreparedTimeoutPhases {
        connect: None,
        read: None,
        write: None,
        pool: None,
    });
    assert_eq!(request::timeout_duration(&absent), None);
    assert_eq!(
        request::timeout_duration(&PreparedTimeout::Seconds(-1.0_f64)),
        None
    );
    assert_eq!(
        request::timeout_duration(&PreparedTimeout::Seconds(f64::INFINITY)),
        None
    );
}

#[tokio::test]
async fn target_stream_over_the_body_limit_fails_typed() {
    let frame = "data: {\"id\":\"chunk-1\"}\n\n";
    let mut fits = Harness::new();
    fits.chat.streams(&[frame]);
    fits.stream_bytes = frame.len();
    let fitting = fits.target();
    let outcome =
        Box::pin(fitting.run(PreparedCompletion::Dispatch(streaming_call()), false, None))
            .await
            .unwrap();
    let CallOutcome::ChatStream(stream) = outcome else {
        panic!("a stream within the limit answers a chat stream");
    };
    let Json::List(chunks) = &stream.model_response else {
        panic!("the stream holds its chunks");
    };
    assert_eq!(chunks.len(), 1);
    let mut over = Harness::new();
    over.chat.streams(&[frame]);
    let room = frame.len().saturating_sub(1);
    over.stream_bytes = room;
    let bounded = over.target();
    let error = Box::pin(bounded.run(PreparedCompletion::Dispatch(streaming_call()), false, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CompletionCallError::Http(HttpError::TooLarge { limit }) if limit == room
    ));
}

#[tokio::test]
async fn target_missing_model_answers_a_bad_request() {
    let harness = Harness::new();
    let target = harness.target();
    let call = request(&format!(r#"{{"messages":[{USER}]}}"#));
    let error = target.call(call, None).await.unwrap_err();
    assert_eq!(class_name(&error.class), "BadRequestError");
    assert_eq!(class_status(&error.class), Some(400));
}

// ---- async_ ----

#[tokio::test]
async fn async_text_provider_keeps_a_chat_shaped_answer() {
    let harness = Harness::new();
    let target = harness.async_target();
    let outcome = target
        .convert_text_reply(
            Some("text-completion-openai"),
            CallOutcome::Response(json(&chat_body("kept"))),
        )
        .unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("the provider's chat reply stays a response");
    };
    assert_eq!(
        dig(answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-agentic")))
    );
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("kept")))
    );
}

#[tokio::test]
async fn async_mock_timeout_is_a_timeout_error() {
    let harness = Harness::new();
    let target = harness.async_target();
    let call = request(&format!(
        r#"{{"model":"gpt-4o","messages":[{USER}],"mock_timeout":true,"timeout":0}}"#
    ));
    let error = target.call(call, None).await.unwrap_err();
    assert_eq!(class_name(&error.class), "Timeout");
    assert_eq!(class_status(&error.class), Some(408));
    assert!(error.to_string().contains("Timeout Error:"), "{error}");
}

use std::time::Duration;

use betterlite_core::dispatch::error::DispatchError;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;

use super::support::{Harness, args_from, dig, prepared_call, request};
use super::{USER, answer_choices, body_text};
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::{class_name, class_status};
use crate::call_wrapper::target::CallTarget as _;
use crate::completion_call::error::{CompletionCallError, into_call_error};
use crate::completion_call::request;
use crate::completion_call::target::MAX_MODEL_LIST_DEPLOYMENTS;
use crate::completion_request::args::JsonObject;
use crate::completion_request::call::{
    MockCall, ModelListCall, PreparedCall, PreparedCompletion, PreparedTimeout,
    PreparedTimeoutPhases,
};

fn streaming_call() -> PreparedCall {
    let mut call = prepared_call("gpt-4o");
    call.stream = true;
    call
}

#[tokio::test]
async fn target_dispatch_returns_the_finished_response() {
    let harness = Harness::new();
    let target = harness.target();
    let call = request(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let outcome = target.call(call, None).await.unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("a plain call answers a finished response");
    };
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("hello")))
    );
    let seen = harness.chat.seen();
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].stream);
    assert_eq!(seen[0].model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn target_stream_call_returns_the_chat_stream() {
    let harness = Harness::new();
    harness.chat.streams(&[
        "data: {\"id\":\"chunk-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"he\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chunk-2\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    ]);
    let target = harness.target();
    let call = request(&format!(
        r#"{{"model":"gpt-4o","messages":[{USER}],"stream":true}}"#
    ));
    let outcome = target.call(call, None).await.unwrap();
    let CallOutcome::ChatStream(stream) = outcome else {
        panic!("a streaming call answers a chat stream");
    };
    assert!(!stream.converted_from_mock);
    let Json::List(chunks) = &stream.model_response else {
        panic!("the stream holds its chunks");
    };
    assert_eq!(chunks.len(), 2);
    assert!(
        dig(&chunks[1], &["choices"]).is_some(),
        "the second chunk is the provider's"
    );
    assert!(harness.chat.seen()[0].stream);
}

#[tokio::test]
async fn target_unknown_model_fails_as_a_bad_request() {
    let harness = Harness::new();
    let target = harness.target();
    let call = request(&format!(
        r#"{{"model":"missing-model","messages":[{USER}]}}"#
    ));
    let error = target.call(call, None).await.unwrap_err();
    assert_eq!(class_name(&error.class), "BadRequestError");
    assert_eq!(class_status(&error.class), Some(400));
}

#[tokio::test]
async fn target_model_list_skips_a_failed_deployment() {
    let harness = Harness::new();
    let target = harness.target();
    let deployments = vec![
        JsonObject::from([(
            String::from("model"),
            Json::Text(String::from("missing-model")),
        )]),
        JsonObject::from([(String::from("model"), Json::Text(String::from("gpt-4o")))]),
    ];
    let call = ModelListCall {
        deployments,
        args: args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#)),
    };
    let outcome = Box::pin(target.run(PreparedCompletion::ModelList(call), false, None))
        .await
        .unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("the second deployment answers");
    };
    assert_eq!(
        dig(answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-1")))
    );
}

#[tokio::test]
async fn target_model_list_without_an_answer_names_the_failure() {
    let harness = Harness::new();
    let target = harness.target();
    let deployments = vec![JsonObject::from([(
        String::from("model"),
        Json::Text(String::from("missing-model")),
    )])];
    let call = ModelListCall {
        deployments,
        args: args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#)),
    };
    let error = Box::pin(target.run(PreparedCompletion::ModelList(call), false, None))
        .await
        .unwrap_err();
    assert!(matches!(&error, CompletionCallError::Call(_)));
    assert!(error.to_string().contains("missing-model"));
}

#[tokio::test]
async fn target_model_list_at_the_deployment_cap_runs_the_race() {
    let harness = Harness::new();
    let target = harness.target();
    let deployment =
        JsonObject::from([(String::from("model"), Json::Text(String::from("gpt-4o")))]);
    let deployments = vec![deployment; MAX_MODEL_LIST_DEPLOYMENTS];
    let call = ModelListCall {
        deployments,
        args: args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#)),
    };
    let outcome = Box::pin(target.run(PreparedCompletion::ModelList(call), false, None))
        .await
        .unwrap();
    assert!(body_text(&outcome).is_some());
}

#[tokio::test]
async fn target_model_list_over_the_deployment_cap_is_refused() {
    let harness = Harness::new();
    let target = harness.target();
    let too_many = MAX_MODEL_LIST_DEPLOYMENTS.saturating_add(1);
    let deployment =
        JsonObject::from([(String::from("model"), Json::Text(String::from("gpt-4o")))]);
    let deployments = vec![deployment; too_many];
    let call = ModelListCall {
        deployments,
        args: args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#)),
    };
    let error = Box::pin(target.run(PreparedCompletion::ModelList(call), false, None))
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        CompletionCallError::TooManyDeployments { count, maximum }
            if *count == too_many && *maximum == MAX_MODEL_LIST_DEPLOYMENTS
    ));
    assert!(harness.chat.seen().is_empty(), "no provider call starts");
    let mapped = into_call_error(&error, "gpt-4o", "", &target.redactor);
    assert_eq!(class_status(&mapped.class), Some(400));
}

#[tokio::test]
async fn target_mock_answers_from_the_mock_response() {
    let harness = Harness::new();
    let target = harness.target();
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        String::from("mock_response"),
        Json::Text(String::from("hi")),
    );
    let mut prepared = prepared_call("gpt-4o");
    prepared.kwargs = kwargs;
    let call = MockCall {
        call: prepared,
        acompletion: false,
        mock_delay: None,
        prompt_tokens: None,
    };
    let outcome = Box::pin(target.run(PreparedCompletion::Mock(call), false, None))
        .await
        .unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("a mock answers a finished response");
    };
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("hi")))
    );
    assert_eq!(
        dig(answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-call-1")))
    );
    assert!(matches!(
        dig(answer, &["created"]),
        Some(Json::Number(number)) if number.as_u64() == Some(1_700_000_000)
    ));
}

#[tokio::test]
async fn target_mock_stream_is_a_chat_stream() {
    let harness = Harness::new();
    let target = harness.target();
    let mut kwargs = JsonObject::new();
    kwargs.insert(
        String::from("mock_response"),
        Json::Text(String::from("hello")),
    );
    let mut prepared = prepared_call("gpt-4o");
    prepared.kwargs = kwargs;
    prepared.stream = true;
    let call = MockCall {
        call: prepared,
        acompletion: true,
        mock_delay: None,
        prompt_tokens: None,
    };
    let outcome = Box::pin(target.run(PreparedCompletion::Mock(call), true, None))
        .await
        .unwrap();
    let CallOutcome::ChatStream(stream) = outcome else {
        panic!("a streaming mock answers a chat stream");
    };
    assert!(stream.converted_from_mock);
    let Json::List(chunks) = &stream.model_response else {
        panic!("the mock stream holds its chunks");
    };
    assert_eq!(chunks.len(), 2, "five characters in three-character deltas");
}

#[tokio::test]
async fn target_palm_raises_its_value_error() {
    let harness = Harness::new();
    let target = harness.target();
    let mut call = prepared_call("palm");
    call.custom_llm_provider = Some(String::from("palm"));
    let error = Box::pin(target.run(PreparedCompletion::Dispatch(call), false, None))
        .await
        .unwrap_err();
    assert!(matches!(&error, CompletionCallError::Palm));
    let mapped = into_call_error(&error, "palm", "", &target.redactor);
    assert!(
        mapped
            .to_string()
            .contains("Palm was decommi\u{73}ioned on October 2024")
    );
}

#[tokio::test]
async fn target_mcp_loop_without_a_runner_fails_by_name() {
    let harness = Harness::new();
    let target = harness.target();
    let prepared = PreparedCompletion::McpLoop(PreparedCall::default());
    let error = Box::pin(target.run(prepared, false, None))
        .await
        .unwrap_err();
    assert!(matches!(&error, CompletionCallError::NoMcpLoop));
}

#[tokio::test]
async fn target_responses_bridge_reaches_the_responses_entry() {
    let harness = Harness::new();
    let target = harness.target();
    let prepared = PreparedCompletion::ResponsesBridge(PreparedCall::default());
    let error = Box::pin(target.run(prepared, false, None))
        .await
        .unwrap_err();
    let CompletionCallError::Dispatch(DispatchError::NotBuilt { entry }) = error else {
        panic!("the bridge goes to the responses entry");
    };
    assert_eq!(entry, "responses");
}

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

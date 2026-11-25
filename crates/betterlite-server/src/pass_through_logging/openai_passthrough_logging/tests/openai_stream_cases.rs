use std::sync::Arc;

use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_core::routes::CallType;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use betterlite_logging::passthrough_handler::types::{
    PASSTHROUGH_LOGGING_PAYLOAD, StreamedResponse,
};
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler as _, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;
use tracing::Level;

use super::super::stream::passthrough_kwargs;
use super::super::tools::{OpenAiLoggingTools as _, StandardObjectRequest};
use super::support::{
    CHAT, Call, FakeTools, RESPONSES, body_user, chat_price, collect, detail, fallback_payload,
    key, object_json, openai_logger, streamed, text, toolbox,
};
use crate::pass_through_logging::base_passthrough_logging::tests::support::{
    END, START, capture_logs, logging_state, object, timestamp,
};
use crate::pass_through_logging::cohere_passthrough_logging::tests::support::RecordingPricing;
use crate::pass_through_logging::gemini_passthrough_logging::tests::support::collected;
use crate::pass_through_logging::response_payload::ResponseLoggingKwargs;

#[test]
fn openai_stream_responses_route_prices_the_terminal_reply() {
    let (logs, _guard) = capture_logs(Level::DEBUG);
    let (tools, call_log, outcome) = streamed(
        FakeTools::working(),
        r#"{"model": "gpt-4o-mini"}"#,
        RESPONSES,
    );
    let reply = object_json(r#"{"id": "resp-2"}"#);
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ResponsesApi(reply.clone()))
    );
    assert_eq!(
        tools.calls(),
        vec![
            Call::Terminal,
            Call::Cost {
                model: "gpt-4o-mini".to_owned(),
                provider: "openai".to_owned(),
                call_type: Some(CallType::Responses),
                response: reply.clone(),
            },
            Call::Standard {
                response: reply,
                logged_cost: Some(decimal_json(chat_price())),
            },
        ]
    );
    assert_eq!(outcome.kwargs.response_cost, Some(chat_price()));
    assert_eq!(outcome.kwargs.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(
        outcome.kwargs.standard_logging_object,
        Some(object_json(r#"{"status": "success"}"#))
    );
    assert_eq!(call_log.model, "gpt-4o-mini");
    assert_eq!(
        detail(&call_log, "custom_llm_provider"),
        Some(text("openai"))
    );
    assert!(logs.contents().contains(
        "OpenAI streaming passthrough cost tracking - Model: gpt-4o-mini, Cost: $0.001500"
    ));
}

#[rstest]
#[case::chat(
    StreamedResponse::Model(object_json(r#"{"id": "s-1"}"#)),
    PassThroughResponse::ModelResponse(object_json(r#"{"id": "s-1"}"#))
)]
#[case::text(
    StreamedResponse::TextCompletion(object_json(r#"{"id": "t-1"}"#)),
    PassThroughResponse::TextCompletion(object_json(r#"{"id": "t-1"}"#))
)]
fn openai_stream_chat_route_assembles_the_chunks(
    #[case] built: StreamedResponse,
    #[case] expected: PassThroughResponse,
) {
    let fake = FakeTools {
        streamed: Some(Some(built)),
        ..FakeTools::working()
    };
    let (tools, _call_log, outcome) = streamed(fake, r#"{"model": "gpt-4o"}"#, CHAT);
    assert_eq!(outcome.result, Some(expected));
    let calls = tools.calls();
    assert_eq!(
        calls.first(),
        Some(&Call::Stream {
            model: "gpt-4o".to_owned()
        })
    );
    assert!(matches!(
        calls.get(1),
        Some(Call::Cost {
            call_type: None,
            ..
        })
    ));
}

#[rstest]
#[case::absent("{}", "gpt-4o")]
#[case::null(r#"{"model": null}"#, "gpt-4o")]
#[case::empty(r#"{"model": ""}"#, "")]
fn openai_stream_model_defaults_to_gpt_4o(#[case] request: &str, #[case] expected: &str) {
    let (tools, _call_log, outcome) = streamed(FakeTools::working(), request, CHAT);
    assert_eq!(
        tools.calls().first(),
        Some(&Call::Stream {
            model: expected.to_owned()
        })
    );
    assert_eq!(outcome.kwargs.model.as_deref(), Some(expected));
}

#[test]
fn openai_stream_provider_comes_from_the_call_log() {
    let tools = Arc::new(FakeTools::working());
    let logger = openai_logger(&tools, &fallback_payload());
    let mut call_log = logging_state();
    call_log
        .model_call_details
        .insert("custom_llm_provider".to_owned(), text("azure"));
    let _outcome = collect(&logger, &mut call_log, r#"{"model": "gpt-4o"}"#, CHAT);
    assert!(
        matches!(tools.calls().get(1), Some(Call::Cost { provider, .. }) if provider == "azure")
    );
    assert_eq!(call_log.custom_llm_provider.as_deref(), Some("azure"));
}

#[rstest]
#[case::no_terminal_reply(RESPONSES)]
#[case::no_assembled_reply(CHAT)]
fn openai_stream_without_a_complete_reply_answers_empty(#[case] url_route: &str) {
    let (logs, _guard) = capture_logs(Level::WARN);
    let tools = FakeTools {
        terminal: Some(None),
        streamed: Some(None),
        ..FakeTools::working()
    };
    let (_tools, _call_log, outcome) = streamed(tools, r#"{"model": "gpt-4o"}"#, url_route);
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, ResponseLoggingKwargs::default());
    assert!(
        logs.contents()
            .contains("Failed to build complete response from OpenAI streaming chunks")
    );
}

#[rstest]
#[case::cost_fails(true, r#"{"model": "gpt-4o"}"#)]
#[case::model_type(false, r#"{"model": 5}"#)]
fn openai_stream_errors_answer_empty(#[case] cost_fails: bool, #[case] request: &str) {
    let (logs, _guard) = capture_logs(Level::ERROR);
    let tools = FakeTools {
        price: (!cost_fails).then_some(chat_price()),
        ..FakeTools::working()
    };
    let (_tools, call_log, outcome) = streamed(tools, request, CHAT);
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, ResponseLoggingKwargs::default());
    assert_eq!(detail(&call_log, "response_cost"), None);
    assert!(
        logs.contents()
            .contains("Error in OpenAI streaming passthrough cost tracking")
    );
}

#[test]
fn openai_stream_user_comes_from_the_logged_payload() {
    let tools = Arc::new(FakeTools::working());
    let logger = openai_logger(&tools, &fallback_payload());
    let mut call_log = logging_state();
    call_log.model_call_details.insert(
        PASSTHROUGH_LOGGING_PAYLOAD.to_owned(),
        object_json(r#"{"request_body": {"user": "user-6"}}"#),
    );
    call_log.litellm_params = object(r#"{"api_base": "x", "proxy_server_request": {"body": {}}}"#);
    let outcome = collect(&logger, &mut call_log, r#"{"model": "gpt-4o"}"#, CHAT);
    assert_eq!(
        outcome.kwargs.litellm_params.get("api_base"),
        Some(&text("x"))
    );
    assert_eq!(
        body_user(&outcome.kwargs.litellm_params),
        Some(text("user-6"))
    );
    assert_eq!(body_user(&call_log.litellm_params), Some(text("user-6")));
}

#[rstest]
#[case::payload_text(text("x"), false)]
#[case::request_body_number(object_json(r#"{"request_body": 5}"#), false)]
#[case::payload_null(Json::Null, true)]
#[case::request_body_null(object_json(r#"{"request_body": null}"#), true)]
fn openai_stream_logged_payload_shapes(#[case] logged: Json, #[case] priced: bool) {
    let tools = Arc::new(FakeTools::working());
    let logger = openai_logger(&tools, &fallback_payload());
    let mut call_log = logging_state();
    call_log
        .model_call_details
        .insert(PASSTHROUGH_LOGGING_PAYLOAD.to_owned(), logged);
    let outcome = collect(&logger, &mut call_log, r#"{"model": "gpt-4o"}"#, CHAT);
    assert_eq!(outcome.result.is_some(), priced);
    assert!(outcome.kwargs.litellm_params.is_empty());
}

#[test]
fn openai_stream_handler_runs_inside_the_logging_object() {
    let tools = Arc::new(FakeTools::working());
    let logger = openai_logger(&tools, &fallback_payload());
    let fixture = collected(RESPONSES, &["data: chunk"], None);
    let request_body = object_json(r#"{"model": "gpt-4o"}"#);
    let outcome = logger
        .handle(&CollectedChunksCall {
            request_body: Some(&request_body),
            ..fixture.call()
        })
        .unwrap();
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ResponsesApi(object_json(
            r#"{"id": "resp-2"}"#
        )))
    );
    let entries = outcome.kwargs.entries;
    assert_eq!(
        entries.get("response_cost"),
        Some(&decimal_json(chat_price()))
    );
    assert_eq!(entries.get("model"), Some(&text("gpt-4o")));
    assert_eq!(
        entries.get(STANDARD_LOGGING_OBJECT),
        Some(&object_json(r#"{"status": "success"}"#))
    );
    let model = fixture
        .logging_obj
        .read(&mut |call_log: &LoggingState| call_log.model.clone());
    assert_eq!(model.as_deref(), Some("gpt-4o"));
}

#[rstest]
#[case::absent(None, true)]
#[case::null(Some(Json::Null), true)]
#[case::text(Some(text("body")), false)]
fn openai_stream_handler_request_body_shapes(
    #[case] request_body: Option<Json>,
    #[case] priced: bool,
) {
    let tools = Arc::new(FakeTools::working());
    let logger = openai_logger(&tools, &fallback_payload());
    let fixture = collected(CHAT, &["data: chunk"], None);
    let outcome = logger
        .handle(&CollectedChunksCall {
            request_body: request_body.as_ref(),
            ..fixture.call()
        })
        .unwrap();
    assert_eq!(outcome.result.is_some(), priced);
    if priced {
        assert_eq!(
            tools.calls().first(),
            Some(&Call::Stream {
                model: "gpt-4o".to_owned()
            })
        );
    } else {
        assert_eq!(outcome, HandlerOutcome::default());
        assert!(tools.calls().is_empty());
    }
}

#[test]
fn openai_stream_kwargs_carry_only_the_set_entries() {
    let full = passthrough_kwargs(ResponseLoggingKwargs {
        response_cost: Some(chat_price()),
        model: Some("gpt-4o".to_owned()),
        litellm_params: object(r#"{"api_base": "x"}"#),
        standard_logging_object: Some(object_json("{}")),
        passthrough_request_body: None,
    });
    let keys: Vec<&str> = full.entries.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            "response_cost",
            "model",
            "litellm_params",
            STANDARD_LOGGING_OBJECT
        ]
    );
    assert!(
        passthrough_kwargs(ResponseLoggingKwargs::default())
            .entries
            .is_empty()
    );
}

#[test]
fn openai_stream_toolbox_assembles_chat_chunks() {
    let tools = toolbox(&Arc::new(RecordingPricing::new(Decimal::ONE)), chat_price());
    let chunk = |delta: &str, finish: &str| {
        format!(
            r#"data: {{"id": "chatcmpl-s", "object": "chat.completion.chunk", "created": 1, "model": "gpt-4o", "choices": [{{"index": 0, "delta": {delta}{finish}}}]}}"#
        )
    };
    let chunks = vec![
        chunk(r#"{"role": "assistant", "content": "Hello"}"#, ""),
        chunk(r#"{"content": " world"}"#, r#", "finish_reason": "stop""#),
    ];
    let Some(StreamedResponse::Model(reply)) = tools.stream_response(&chunks, "gpt-4o").unwrap()
    else {
        panic!("a chat reply");
    };
    let Json::List(choices) = key(&reply, "choices") else {
        panic!("the reply lists its choices");
    };
    assert_eq!(
        key(key(&choices[0], "message"), "content"),
        &text("Hello world")
    );
    assert_eq!(tools.stream_response(&[], "gpt-4o").unwrap(), None);
}

#[test]
fn openai_stream_toolbox_reads_the_terminal_event() {
    let tools = toolbox(&Arc::new(RecordingPricing::new(Decimal::ONE)), chat_price());
    let chunks = vec![r#"data: {"type": "response.completed", "response": {"id": "resp-9", "created_at": 7, "output": []}}"#.to_owned()];
    let reply = tools
        .terminal_response(&chunks)
        .unwrap()
        .expect("a terminal reply");
    assert_eq!(key(&reply, "id"), &text("resp-9"));
    let unfinished = vec![r#"data: {"type": "response.created", "response": {"id": "r", "created_at": 1, "output": []}}"#.to_owned()];
    assert_eq!(tools.terminal_response(&unfinished).unwrap(), None);
}

#[test]
fn openai_stream_toolbox_builds_the_standard_object() {
    let tools = toolbox(&Arc::new(RecordingPricing::new(Decimal::ONE)), chat_price());
    let reply = object_json("{}");
    let call_log = logging_state();
    let request = StandardObjectRequest {
        response: &reply,
        start_time: timestamp(START),
        end_time: timestamp(END),
    };
    let built = tools
        .standard_logging_object(&request, &call_log)
        .expect("a standard object");
    assert_eq!(key(&built, "id"), &text("chatcmpl-1"));
}

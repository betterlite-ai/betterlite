#![cfg(test)]

use std::sync::Arc;

use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use rust_decimal::Decimal;

use crate::pass_through_logging::base_passthrough_logging::tests::support::{
    END, FakePayload, FakeProvider, RecordingBuilder, START, field, json_object, logging_state,
    object, timestamp,
};
use crate::pass_through_logging::base_passthrough_logging::{
    PassthroughLoggingError, PassthroughProvider as _, UpstreamChatReply,
};
use crate::pass_through_logging::response_payload::ResponseLoggingKwargs;

use super::chat::CohereProvider;
use super::{CohereEmbeddingTools as _, EmbeddingDecodeInput, StandardObjectInput};
use support::{
    FakeTools, PricedCall, RecordingPricing, call_log_detail, cohere_logger, embedding_response,
    input, json, production_tools,
};

pub(crate) mod support;

#[test]
fn cohere_route_embed_goes_to_the_embed_branch() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::Embedding(_))
    ));
    assert_eq!(tools.calls(), ["decode", "cost", "standard"]);
    assert!(provider.seen().is_empty());
    assert!(payload.seen().is_empty());
}

#[test]
fn cohere_route_embeddings_goes_to_the_chat_handler() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embeddings",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embeddings route is logged as chat");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
    assert_eq!(provider.seen(), ["command-r"]);
    assert!(tools.calls().is_empty());
}

#[test]
fn cohere_route_embed_prefix_goes_to_the_embed_branch() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed/job-1",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed prefix route is logged");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::Embedding(_))
    ));
    assert_eq!(tools.calls(), ["decode", "cost", "standard"]);
}

#[test]
fn cohere_route_chat_goes_to_the_chat_handler() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v2/chat",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the chat route is logged");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
    assert_eq!(provider.seen(), ["command-r"]);
    assert!(tools.calls().is_empty());
}

#[test]
fn cohere_route_provider_name_is_cohere() {
    assert_eq!(CohereProvider.provider_name(), ProviderId::Cohere);
}

#[test]
fn cohere_embed_takes_texts_before_input() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a", "b"], "input": ["c"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let _outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert_eq!(tools.texts(), [vec!["a".to_owned(), "b".to_owned()]]);
    assert_eq!(tools.models(), ["command-r", "command-r"]);
}

#[test]
fn cohere_embed_falls_back_to_input_for_empty_texts() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": [], "input": ["c"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let _outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert_eq!(tools.texts(), [vec!["c".to_owned()]]);
}

#[test]
fn cohere_embed_falls_back_to_input_for_null_texts() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": null, "input": ["c"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let _outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert_eq!(tools.texts(), [vec!["c".to_owned()]]);
}

#[test]
fn cohere_embed_uses_no_text_when_neither_key_is_present() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r"}"#);
    let response = object("{}");
    let mut state = logging_state();
    let _outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert_eq!(tools.texts(), [Vec::<String>::new()]);
}

#[test]
fn cohere_embed_takes_the_model_from_the_response_body_when_the_request_omits_it() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"texts": ["a"]}"#);
    let response = object(r#"{"model": "served-model"}"#);
    let mut state = logging_state();
    let _outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert_eq!(tools.models(), ["served-model", "served-model"]);
    assert_eq!(tools.texts(), [vec!["a".to_owned()]]);
}

#[test]
fn cohere_embed_falls_back_to_the_chat_handler_for_a_non_list_texts_value() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": "a", "input": ["c"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the chat handler logs the reply");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
    assert!(tools.calls().is_empty());
    assert_eq!(provider.seen(), ["command-r"]);
}

#[test]
fn cohere_embed_falls_back_to_the_chat_handler_for_a_non_list_input_value() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "input": {"text": "c"}}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the chat handler logs the reply");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
    assert!(tools.calls().is_empty());
    assert_eq!(provider.seen(), ["command-r"]);
}

#[test]
fn cohere_embed_stores_the_cost_in_the_hidden_params() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    let Some(PassThroughResponse::Embedding(logged)) = outcome.result else {
        panic!("the result is the embedding response");
    };
    assert_eq!(field(&logged, "object"), &Json::Text("list".to_owned()));
    assert_eq!(
        field(&logged, "model"),
        &Json::Text("embed-model".to_owned())
    );
    let hidden = field(&logged, "_hidden_params");
    assert_eq!(field(hidden, "response_cost"), &json("0.25"));
}

#[test]
fn cohere_embed_sets_the_kwargs_and_the_call_log() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert_eq!(outcome.kwargs.response_cost, Some(Decimal::new(25, 2)));
    assert_eq!(outcome.kwargs.model.as_deref(), Some("command-r"));
    assert_eq!(outcome.kwargs.standard_logging_object, None);
    assert_eq!(
        call_log_detail(&state, "model"),
        Some(Json::Text("command-r".to_owned()))
    );
    assert_eq!(
        call_log_detail(&state, "custom_llm_provider"),
        Some(Json::Text("cohere".to_owned()))
    );
    assert_eq!(call_log_detail(&state, "response_cost"), Some(json("0.25")));
}

#[test]
fn cohere_embed_writes_the_call_log_after_the_standard_object() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let _outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert_eq!(tools.standard_seen(), [false]);
    assert!(call_log_detail(&state, "response_cost").is_some());
}

#[test]
fn cohere_embed_replaces_only_the_proxy_server_request() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let body = object(r#"{"user": "ignored"}"#);
    let payload =
        Arc::new(FakePayload::new(ResponseLoggingKwargs::default()).with_end_user("alice"));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let kwargs = ResponseLoggingKwargs {
        response_cost: None,
        model: None,
        litellm_params: object(r#"{"api_base": "https://api.cohere.ai"}"#),
        standard_logging_object: None,
        passthrough_request_body: Some(body.clone()),
    };
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(&request, &response, &mut state, "/v1/embed", kwargs))
        .expect("the embed reply is logged");
    assert_eq!(
        outcome.kwargs.litellm_params.get("api_base"),
        Some(&Json::Text("https://api.cohere.ai".to_owned()))
    );
    let expected = json_object(r#"{"body": {"user": "alice"}}"#);
    assert_eq!(
        outcome.kwargs.litellm_params.get("proxy_server_request"),
        Some(&expected)
    );
    assert_eq!(payload.end_user_bodies(), [Some(body)]);
}

#[test]
fn cohere_embed_skips_the_user_lookup_without_a_request_body() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload =
        Arc::new(FakePayload::new(ResponseLoggingKwargs::default()).with_end_user("alice"));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the embed reply is logged");
    assert!(
        !outcome
            .kwargs
            .litellm_params
            .contains_key("proxy_server_request")
    );
    assert!(payload.end_user_bodies().is_empty());
}

#[test]
fn cohere_embed_keeps_the_kwargs_without_a_found_user() {
    let tools = Arc::new(FakeTools::priced(Decimal::new(25, 2)));
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let body = object(r#"{"texts": ["a"]}"#);
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let kwargs = ResponseLoggingKwargs {
        response_cost: None,
        model: None,
        litellm_params: object(r#"{"api_base": "https://api.cohere.ai"}"#),
        standard_logging_object: None,
        passthrough_request_body: Some(body.clone()),
    };
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(&request, &response, &mut state, "/v1/embed", kwargs))
        .expect("the embed reply is logged");
    assert_eq!(
        outcome.kwargs.litellm_params.get("api_base"),
        Some(&Json::Text("https://api.cohere.ai".to_owned()))
    );
    assert!(
        !outcome
            .kwargs
            .litellm_params
            .contains_key("proxy_server_request")
    );
    assert_eq!(payload.end_user_bodies(), [Some(body)]);
}

#[test]
fn cohere_fallback_runs_the_chat_handler_after_a_decode_failure() {
    let tools = Arc::new(FakeTools::failing_decode());
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the chat handler logs the reply");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
    assert_eq!(tools.calls(), ["decode"]);
    assert_eq!(provider.seen(), ["command-r"]);
    assert_eq!(payload.seen(), ["command-r"]);
    assert_eq!(payload.inputs()[0].response_cost, None);
}

#[test]
fn cohere_fallback_runs_the_chat_handler_after_a_cost_failure() {
    let tools = Arc::new(FakeTools::failing_cost());
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the chat handler logs the reply");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
    assert_eq!(tools.calls(), ["decode", "cost"]);
    assert_eq!(payload.inputs()[0].response_cost, None);
}

#[test]
fn cohere_fallback_keeps_the_kwargs_the_failed_branch_wrote() {
    let tools = Arc::new(FakeTools::failing_standard());
    let provider = Arc::new(FakeProvider::chat(json_object(r#"{"choices": []}"#)));
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools)
        .cohere_passthrough_handler(input(
            &request,
            &response,
            &mut state,
            "/v1/embed",
            ResponseLoggingKwargs::default(),
        ))
        .expect("the chat handler logs the reply");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
    assert_eq!(tools.calls(), ["decode", "cost", "standard"]);
    let seen = payload.inputs();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].response_cost, Some(Decimal::new(25, 2)));
    assert_eq!(seen[0].model.as_deref(), Some("command-r"));
}

#[test]
fn cohere_fallback_returns_the_chat_error() {
    let tools = Arc::new(FakeTools::failing_decode());
    let provider = Arc::new(FakeProvider::failing());
    let payload = Arc::new(FakePayload::new(ResponseLoggingKwargs::default()));
    let request = object(r#"{"model": "command-r", "texts": ["a"]}"#);
    let response = object("{}");
    let mut state = logging_state();
    let outcome = cohere_logger(&provider, &payload, &tools).cohere_passthrough_handler(input(
        &request,
        &response,
        &mut state,
        "/v1/embed",
        ResponseLoggingKwargs::default(),
    ));
    let Err(error) = outcome else {
        panic!("the chat error comes back to the caller");
    };
    assert!(matches!(error, PassthroughLoggingError::Transform { .. }));
    assert!(payload.seen().is_empty());
}

#[test]
fn cohere_stream_builds_the_text_response() {
    let provider = CohereProvider;
    let chunks = [
        r#"{"text": "hello ", "is_finished": false}"#.to_owned(),
        r#"{"text": "world", "is_finished": false}"#.to_owned(),
        r#"{"is_finished": true, "finish_reason": "COMPLETE"}"#.to_owned(),
    ];
    let built = provider
        .build_complete_streaming_response(&chunks, "command-r")
        .expect("the collected chunks build a response");
    let Some(StreamedResponse::Model(logged)) = built else {
        panic!("a chat stream builds a model response");
    };
    assert_eq!(field(&logged, "model"), &Json::Text("command-r".to_owned()));
    let Json::List(choices) = field(&logged, "choices") else {
        panic!("the response carries choices");
    };
    let choice = &choices[0];
    assert_eq!(
        field(choice, "finish_reason"),
        &Json::Text("stop".to_owned())
    );
    assert_eq!(
        field(field(choice, "message"), "content"),
        &Json::Text("hello world".to_owned())
    );
}

#[test]
fn cohere_stream_returns_none_for_no_chunks() {
    let provider = CohereProvider;
    let chunks: Vec<String> = Vec::new();
    let built = provider
        .build_complete_streaming_response(&chunks, "command-r")
        .expect("no chunks are not an error");
    assert!(built.is_none());
}

#[test]
fn cohere_stream_skips_a_chunk_without_content() {
    let provider = CohereProvider;
    let chunks = [
        r#"{"text": "hello", "is_finished": false}"#.to_owned(),
        "{}".to_owned(),
        r#"{"is_finished": true, "finish_reason": "COMPLETE"}"#.to_owned(),
    ];
    let built = provider
        .build_complete_streaming_response(&chunks, "command-r")
        .expect("an empty chunk is skipped");
    let Some(StreamedResponse::Model(logged)) = built else {
        panic!("a chat stream builds a model response");
    };
    let Json::List(choices) = field(&logged, "choices") else {
        panic!("the response carries choices");
    };
    assert_eq!(
        field(field(&choices[0], "message"), "content"),
        &Json::Text("hello".to_owned())
    );
}

#[test]
fn cohere_stream_stops_at_the_first_end_of_stream() {
    let provider = CohereProvider;
    let chunks = [
        r#"{"text": "hello", "is_finished": false}"#.to_owned(),
        r#"{"is_finished": true, "finish_reason": "COMPLETE"}"#.to_owned(),
        "{}".to_owned(),
        r#"{"text": "tail", "is_finished": false}"#.to_owned(),
    ];
    let built = provider
        .build_complete_streaming_response(&chunks, "command-r")
        .expect("the stream stops after its finish reason");
    let Some(StreamedResponse::Model(logged)) = built else {
        panic!("a chat stream builds a model response");
    };
    let Json::List(choices) = field(&logged, "choices") else {
        panic!("the response carries choices");
    };
    assert_eq!(
        field(field(&choices[0], "message"), "content"),
        &Json::Text("hello".to_owned())
    );
}

#[test]
fn cohere_stream_returns_the_decode_error() {
    let provider = CohereProvider;
    let chunks = ["not json".to_owned()];
    let outcome = provider.build_complete_streaming_response(&chunks, "command-r");
    let Err(error) = outcome else {
        panic!("an invalid line fails the build");
    };
    assert!(matches!(error, PassthroughLoggingError::StreamBuild { .. }));
}

#[test]
fn cohere_stream_returns_the_decode_error_for_a_data_prefixed_line() {
    let provider = CohereProvider;
    let chunks = [r#"data: {"text": "hello", "is_finished": false}"#.to_owned()];
    let outcome = provider.build_complete_streaming_response(&chunks, "command-r");
    let Err(error) = outcome else {
        panic!("a line with the SSE prefix fails the build");
    };
    assert!(matches!(error, PassthroughLoggingError::StreamBuild { .. }));
}

#[test]
fn cohere_stream_returns_the_missing_finish_reason_error() {
    let provider = CohereProvider;
    let chunks = [r#"{"is_finished": true}"#.to_owned()];
    let outcome = provider.build_complete_streaming_response(&chunks, "command-r");
    let Err(error) = outcome else {
        panic!("a finished chunk without a reason fails the build");
    };
    assert!(matches!(error, PassthroughLoggingError::StreamBuild { .. }));
}

const CHAT_REPLY: &str = r#"{"id": "reply-1", "message": {"content": [{"type": "text", "text": "hello "}, {"type": "text", "text": "world"}], "citations": [{"start": 0, "end": 5, "sources": [{"type": "document", "id": "doc-1", "url": "https://docs.test/a", "document": {"title": "Doc"}}]}]}, "usage": {"tokens": {"input_tokens": 3, "output_tokens": 2}}, "finish_reason": "COMPLETE"}"#;
const TOOL_REPLY: &str = r#"{"message": {"content": [{"type": "text", "text": "dropped"}], "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "lookup", "arguments": "{\"q\": 1}"}}], "citations": [{"start": 0, "end": 1, "sources": []}]}, "usage": {"tokens": {"input_tokens": 1}}}"#;
const PLAIN_REPLY: &str =
    r#"{"message": {"content": [{"type": "text", "text": "hi"}]}, "usage": {"tokens": {}}}"#;

fn transformed(body: &str) -> Json {
    CohereProvider
        .transform_chat_response(
            "command-r",
            &UpstreamChatReply {
                status_code: 200,
                body: body.as_bytes(),
            },
        )
        .expect("the v2 reply is transformed")
}

#[test]
fn cohere_route_chat_transforms_the_v2_reply_into_a_logged_model_response() {
    let response = transformed(CHAT_REPLY);
    assert_eq!(
        field(&response, "object"),
        &Json::Text("chat.completion".to_owned())
    );
    assert_eq!(
        field(&response, "model"),
        &Json::Text("command-r".to_owned())
    );
    assert_eq!(field(&response, "system_fingerprint"), &Json::Null);
    let Json::Text(id) = field(&response, "id") else {
        panic!("the id is a string");
    };
    assert!(id.starts_with("chatcmpl-"));
    let Json::Number(created) = field(&response, "created") else {
        panic!("created is a number");
    };
    assert!(
        created
            .as_u64()
            .is_some_and(|seconds| seconds > 1_700_000_000)
    );
    let Json::List(choices) = field(&response, "choices") else {
        panic!("choices is a list");
    };
    assert_eq!(choices.len(), 1);
    let choice = &choices[0];
    assert_eq!(
        field(choice, "finish_reason"),
        &Json::Text("stop".to_owned())
    );
    assert_eq!(field(choice, "index"), &json("0"));
}

#[test]
fn cohere_route_chat_logs_the_message_and_usage() {
    let response = transformed(CHAT_REPLY);
    let Json::List(choices) = field(&response, "choices") else {
        panic!("choices is a list");
    };
    let message = field(&choices[0], "message");
    assert_eq!(
        field(message, "content"),
        &Json::Text("hello world".to_owned())
    );
    assert_eq!(field(message, "role"), &Json::Text("assistant".to_owned()));
    assert_eq!(field(message, "function_call"), &Json::Null);
    assert_eq!(field(message, "tool_calls"), &Json::Null);
    assert_eq!(
        field(message, "annotations"),
        &json(
            r#"[{"type": "url_citation", "url_citation": {"start_index": 0, "end_index": 5, "title": "Doc", "url": "https://docs.test/a"}}]"#
        )
    );
    let usage = field(&response, "usage");
    assert_eq!(field(usage, "prompt_tokens"), &json("3"));
    assert_eq!(field(usage, "completion_tokens"), &json("2"));
    assert_eq!(field(usage, "total_tokens"), &json("5"));
}

#[test]
fn cohere_route_chat_keeps_the_tool_call_reply_and_its_empty_annotations() {
    let response = transformed(TOOL_REPLY);
    let Json::List(choices) = field(&response, "choices") else {
        panic!("choices is a list");
    };
    let message = field(&choices[0], "message");
    assert_eq!(field(message, "content"), &Json::Null);
    assert_eq!(field(message, "annotations"), &json("[]"));
    let Json::List(calls) = field(message, "tool_calls") else {
        panic!("tool_calls is a list");
    };
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(field(call, "index"), &json("0"));
    assert_eq!(field(call, "id"), &Json::Text("call-1".to_owned()));
    assert_eq!(field(call, "type"), &Json::Text("function".to_owned()));
    let function = field(call, "function");
    assert_eq!(field(function, "name"), &Json::Text("lookup".to_owned()));
    assert_eq!(
        field(function, "arguments"),
        &Json::Text(r#"{"q": 1}"#.to_owned())
    );
}

#[test]
fn cohere_route_chat_omits_annotations_when_the_reply_has_none() {
    let response = transformed(PLAIN_REPLY);
    let Json::List(choices) = field(&response, "choices") else {
        panic!("choices is a list");
    };
    let message = field(&choices[0], "message");
    assert_eq!(field(message, "content"), &Json::Text("hi".to_owned()));
    assert_eq!(field(message, "tool_calls"), &Json::Null);
    let Json::Object(fields) = message else {
        panic!("the message is an object");
    };
    assert!(!fields.contains_key("annotations"));
}

const EMBED_REPLY: &[u8] = br#"{"id": "embed-1", "embeddings": [[0.5, 0.25]], "meta": {"billed_units": {"input_tokens": 3}}}"#;

#[test]
fn cohere_embed_tools_decode_logs_the_reply_and_fills_the_response() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::ZERO));
    let builder = Arc::new(RecordingBuilder::default());
    let request = object(r#"{"model": "embed-english-v3.0", "texts": ["hello"]}"#);
    let texts = [Json::Text("hello".to_owned())];
    let mut state = logging_state();
    let response = production_tools(&pricing, &builder)
        .decode_embedding(
            EmbeddingDecodeInput {
                upstream_body: EMBED_REPLY,
                request_body: &request,
                model: "embed-english-v3.0",
                input: &texts,
            },
            &mut state,
        )
        .expect("the embedding reply decodes");
    assert_eq!(response.object, "list");
    assert_eq!(response.model, "embed-english-v3.0");
    assert_eq!(
        response.data,
        [json_object(
            r#"{"object": "embedding", "index": 0, "embedding": [0.5, 0.25]}"#
        )]
    );
    assert_eq!(response.usage.prompt_tokens, 3);
    assert!(response.hidden_params.is_empty());
}

#[test]
fn cohere_embed_tools_decode_logs_the_post_call() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::ZERO));
    let builder = Arc::new(RecordingBuilder::default());
    let request = object(r#"{"model": "embed-english-v3.0", "texts": ["hello"]}"#);
    let texts = [Json::Text("hello".to_owned())];
    let mut state = logging_state();
    let _response = production_tools(&pricing, &builder)
        .decode_embedding(
            EmbeddingDecodeInput {
                upstream_body: EMBED_REPLY,
                request_body: &request,
                model: "embed-english-v3.0",
                input: &texts,
            },
            &mut state,
        )
        .expect("the embedding reply decodes");
    assert_eq!(
        call_log_detail(&state, "log_event_type"),
        Some(Json::Text("post_api_call".to_owned()))
    );
    assert_eq!(call_log_detail(&state, "input"), Some(json(r#"["hello"]"#)));
    let Some(Json::Text(original)) = call_log_detail(&state, "original_response") else {
        panic!("the parsed reply is logged as text");
    };
    assert!(original.contains("embed-1"));
    let Some(arguments) = call_log_detail(&state, "additional_args") else {
        panic!("the post_call arguments are logged");
    };
    assert_eq!(
        field(&arguments, "complete_input_dict"),
        &Json::Object(request)
    );
}

#[test]
fn cohere_embed_tools_decode_logs_the_reply_before_a_missing_embeddings_error() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::ZERO));
    let builder = Arc::new(RecordingBuilder::default());
    let request = object(r#"{"model": "embed-english-v3.0"}"#);
    let mut state = logging_state();
    let outcome = production_tools(&pricing, &builder).decode_embedding(
        EmbeddingDecodeInput {
            upstream_body: br#"{"id": "embed-2"}"#,
            request_body: &request,
            model: "embed-english-v3.0",
            input: &[],
        },
        &mut state,
    );
    let Err(error) = outcome else {
        panic!("a reply without embeddings fails the decode");
    };
    assert!(matches!(error, PassthroughLoggingError::Transform { .. }));
    let Some(Json::Text(original)) = call_log_detail(&state, "original_response") else {
        panic!("the parsed reply is logged before the error");
    };
    assert!(original.contains("embed-2"));
}

#[test]
fn cohere_embed_tools_decode_rejects_a_text_that_is_not_a_string() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::ZERO));
    let builder = Arc::new(RecordingBuilder::default());
    let request = object(r#"{"model": "embed-english-v3.0"}"#);
    let texts = [json("1")];
    let mut state = logging_state();
    let outcome = production_tools(&pricing, &builder).decode_embedding(
        EmbeddingDecodeInput {
            upstream_body: EMBED_REPLY,
            request_body: &request,
            model: "embed-english-v3.0",
            input: &texts,
        },
        &mut state,
    );
    let Err(error) = outcome else {
        panic!("a numeric text fails the decode");
    };
    assert!(matches!(error, PassthroughLoggingError::Transform { .. }));
    assert_eq!(call_log_detail(&state, "original_response"), None);
}

#[test]
fn cohere_embed_tools_price_the_embedding_as_a_cohere_aembedding() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::new(3, 4)));
    let builder = Arc::new(RecordingBuilder::default());
    let cost = production_tools(&pricing, &builder)
        .embedding_cost(&embedding_response(), "embed-english-v3.0")
        .expect("the embedding is priced");
    assert_eq!(cost, Decimal::new(3, 4));
    assert_eq!(
        pricing.calls(),
        [PricedCall {
            model: "embed-english-v3.0".to_owned(),
            provider: "cohere".to_owned(),
            call_type: CallType::Aembedding,
            response_model: Some("embed-model".to_owned()),
        }]
    );
}

#[test]
fn cohere_embed_tools_hand_the_response_to_the_standard_object_builder() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::ZERO));
    let builder = Arc::new(RecordingBuilder::default());
    let mut response = embedding_response();
    response
        .hidden_params
        .insert("response_cost".to_owned(), json("0.25"));
    let kwargs = ResponseLoggingKwargs::default();
    let mut state = logging_state();
    production_tools(&pricing, &builder)
        .standard_logging_object(
            StandardObjectInput {
                kwargs: &kwargs,
                response: &response,
                start_time: timestamp(START),
                end_time: timestamp(END),
            },
            &mut state,
        )
        .expect("the standard object step succeeds");
    let seen = builder.responses();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        field(&seen[0], "model"),
        &Json::Text("embed-model".to_owned())
    );
    assert_eq!(
        field(field(&seen[0], "_hidden_params"), "response_cost"),
        &json("0.25")
    );
}

#![cfg(test)]

// The streaming entry's cases; the module name keeps them under the openai_stream_ filter.
mod openai_stream_cases;
pub(crate) mod support;

use std::sync::Arc;

use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;
use tracing::Level;

use super::tools::{
    ChatTransformRequest, ImageEndpoint, OpenAiLoggingTools as _, OpenAiToolsError, PricedResponse,
};
use super::user::{OpenAiPayload, openai_end_user};
use super::{OpenAiPassthroughError, OpenAiPassthroughLogger};
use crate::pass_through_logging::base_passthrough_logging::tests::support::{
    END, FixedCost, RecordingBuilder, START, capture_logs, logging_state, object, timestamp,
};
use crate::pass_through_logging::base_passthrough_logging::{
    PassthroughLoggingError, PassthroughPayload as _, PassthroughPayloadInput, UpstreamChatReply,
};
use crate::pass_through_logging::cohere_passthrough_logging::tests::support::{
    PricedCall, RecordingPricing,
};
use crate::pass_through_logging::response_payload::ResponseLoggingKwargs;
use support::{
    CHAT, Call, EDITS, EMBEDDING, EMBEDDINGS, FakeTools, IMAGES, RESPONSES, Relay, body_user,
    chat_price, detail, fallback_payload, key, object_json, openai_logger, responses_reply, run,
    text, toolbox,
};

const IMAGE_CALL_TYPE: &str = "passthrough-image-generation";

#[test]
fn openai_route_unknown_route_returns_the_kwargs_untouched() {
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, "{}");
    relay.kwargs.response_cost = Some(Decimal::new(9, 0));
    let (tools, outcome) = run(
        FakeTools::working(),
        &mut relay,
        "https://api.openai.com/v1/models",
    );
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, relay.kwargs);
    assert!(tools.calls().is_empty());
}

#[rstest]
#[case::chat(CHAT, "chat_completions")]
#[case::embeddings(EMBEDDINGS, "embeddings")]
#[case::image_generation(IMAGES, "image_generation")]
#[case::image_editing(EDITS, "image_editing")]
#[case::responses(RESPONSES, "responses")]
fn openai_route_each_endpoint_logs_its_name_model_and_cost(
    #[case] url_route: &str,
    #[case] name: &str,
) {
    let (logs, _guard) = capture_logs(Level::DEBUG);
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, r#"{"data": []}"#);
    let (_tools, outcome) = run(FakeTools::working(), &mut relay, url_route);
    assert!(outcome.result.is_some());
    let contents = logs.contents();
    assert!(
        contents.contains(&format!(
            "OpenAI passthrough cost tracking - Endpoint: {name}, Model: gpt-4o, Cost: $"
        )),
        "{contents}"
    );
}

#[rstest]
#[case::chat(CHAT, "chat")]
#[case::embeddings(EMBEDDINGS, "embedding")]
#[case::image_generation(IMAGES, "generation")]
#[case::image_editing(EDITS, "editing")]
#[case::responses(RESPONSES, "responses")]
fn openai_route_picks_the_endpoint_branch(#[case] url_route: &str, #[case] branch: &str) {
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, r#"{"data": []}"#);
    let (tools, _outcome) = run(FakeTools::working(), &mut relay, url_route);
    let first = tools.calls().first().cloned().expect("a tool was called");
    let taken = match first {
        Call::Chat { .. } => "chat",
        Call::Embedding => "embedding",
        Call::ImageCost {
            endpoint: ImageEndpoint::Generation,
            ..
        } => "generation",
        Call::ImageCost {
            endpoint: ImageEndpoint::Editing,
            ..
        } => "editing",
        Call::Responses { .. } => "responses",
        Call::EmbeddingCost { .. }
        | Call::Terminal
        | Call::Stream { .. }
        | Call::Cost { .. }
        | Call::Standard { .. } => "other",
    };
    assert_eq!(taken, branch);
}

#[test]
fn openai_model_request_model_wins_over_the_response_model() {
    let mut relay = Relay::new(
        r#"{"model": "text-embedding-3-small"}"#,
        r#"{"model": "served-model"}"#,
    );
    let (tools, outcome) = run(FakeTools::working(), &mut relay, EMBEDDINGS);
    assert_eq!(
        outcome.kwargs.model.as_deref(),
        Some("text-embedding-3-small")
    );
    assert!(tools.calls().contains(&Call::EmbeddingCost {
        model: "text-embedding-3-small".to_owned(),
        provider: "openai".to_owned(),
    }));
}

#[test]
fn openai_model_response_model_is_used_when_the_request_names_none() {
    let mut relay = Relay::new("{}", r#"{"model": "served-model"}"#);
    let (_tools, outcome) = run(FakeTools::working(), &mut relay, EMBEDDINGS);
    assert_eq!(outcome.kwargs.model.as_deref(), Some("served-model"));
    assert_eq!(relay.call_log.model, "served-model");
}

#[rstest]
#[case::nothing(EMBEDDINGS, "{}", "{}", "")]
#[case::empty_request_model(CHAT, r#"{"model": ""}"#, r#"{"model": "gpt-4o"}"#, "")]
#[case::null_request_model(IMAGES, r#"{"model": null}"#, r#"{"model": "gpt-4o"}"#, "gpt-4o")]
#[case::empty_response_model(RESPONSES, "{}", r#"{"model": ""}"#, "")]
fn openai_model_missing_model_falls_back_to_the_chat_handler(
    #[case] url_route: &str,
    #[case] request: &str,
    #[case] response: &str,
    #[case] fallback_model: &str,
) {
    let (logs, _guard) = capture_logs(Level::WARN);
    let tools = Arc::new(FakeTools::working());
    let payload = fallback_payload();
    let mut relay = Relay::new(request, response);
    let outcome = relay
        .run(&openai_logger(&tools, &payload), url_route)
        .unwrap();
    assert!(
        logs.contents()
            .contains("No model found in request or response for OpenAI passthrough cost tracking")
    );
    assert_eq!(payload.seen(), vec![fallback_model.to_owned()]);
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
    assert_eq!(
        tools.calls(),
        vec![Call::Chat {
            model: fallback_model.to_owned(),
            messages: Json::List(Vec::new()),
            optional_params: Json::Object(JsonObject::new()),
            request_data: JsonObject::new(),
            json_mode: false,
            litellm_params: JsonObject::new(),
        }]
    );
}

#[rstest]
#[case::request(r#"{"model": 5}"#, "{}", "request_body.model")]
#[case::response("{}", r#"{"model": ["gpt-4o"]}"#, "response_body.model")]
fn openai_model_of_the_wrong_type_is_an_error_naming_the_field(
    #[case] request: &str,
    #[case] response: &str,
    #[case] expected: &str,
) {
    let tools = Arc::new(FakeTools::working());
    let mut relay = Relay::new(request, response);
    let error = relay
        .run(&openai_logger(&tools, &fallback_payload()), CHAT)
        .unwrap_err();
    assert!(
        matches!(error, OpenAiPassthroughError::FieldType { field } if field == expected),
        "{error}"
    );
}

#[test]
fn openai_chat_transform_receives_the_request_fields() {
    let request = r#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "optional_params": {"temperature": 0.5}, "response_format": {"type": "json_object"}}"#;
    let mut relay = Relay::new(request, "{}");
    relay.kwargs.litellm_params = object(r#"{"api_base": "https://api.openai.com"}"#);
    let (tools, _outcome) = run(FakeTools::working(), &mut relay, CHAT);
    assert_eq!(
        tools.calls().first(),
        Some(&Call::Chat {
            model: "gpt-4o".to_owned(),
            messages: Json::List(vec![object_json(r#"{"role": "user", "content": "hi"}"#)]),
            optional_params: object_json(r#"{"temperature": 0.5}"#),
            request_data: object(request),
            json_mode: true,
            litellm_params: object(r#"{"api_base": "https://api.openai.com"}"#),
        })
    );
}

#[rstest]
#[case::absent(r#"{"model": "gpt-4o"}"#, false)]
#[case::null(r#"{"model": "gpt-4o", "response_format": null}"#, false)]
#[case::json_object(
    r#"{"model": "gpt-4o", "response_format": {"type": "json_object"}}"#,
    true
)]
#[case::text(r#"{"model": "gpt-4o", "response_format": {"type": "text"}}"#, false)]
#[case::no_type(r#"{"model": "gpt-4o", "response_format": {}}"#, false)]
#[case::number_type(r#"{"model": "gpt-4o", "response_format": {"type": 5}}"#, false)]
fn openai_chat_json_mode_reads_the_response_format_type(
    #[case] request: &str,
    #[case] expected: bool,
) {
    let mut relay = Relay::new(request, "{}");
    let (tools, _outcome) = run(FakeTools::working(), &mut relay, CHAT);
    let Some(Call::Chat { json_mode, .. }) = tools.calls().first().cloned() else {
        panic!("the chat transform ran first");
    };
    assert_eq!(json_mode, expected);
}

#[test]
fn openai_chat_null_messages_and_params_default_to_empty() {
    let mut relay = Relay::new(
        r#"{"model": "gpt-4o", "messages": null, "optional_params": null}"#,
        "{}",
    );
    let (tools, _outcome) = run(FakeTools::working(), &mut relay, CHAT);
    let Some(Call::Chat {
        messages,
        optional_params,
        json_mode,
        ..
    }) = tools.calls().first().cloned()
    else {
        panic!("the chat transform ran first");
    };
    assert_eq!(messages, Json::List(Vec::new()));
    assert_eq!(optional_params, Json::Object(JsonObject::new()));
    assert!(!json_mode);
}

#[test]
fn openai_chat_prices_the_reply_and_fills_the_kwargs_and_call_log() {
    let (logs, _guard) = capture_logs(Level::DEBUG);
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, "{}");
    relay.kwargs.passthrough_request_body = Some(object(r#"{"model": "gpt-4o"}"#));
    let (_tools, outcome) = run(FakeTools::working(), &mut relay, CHAT);
    let reply = object_json(r#"{"id": "chatcmpl-1", "choices": []}"#);
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(reply))
    );
    assert_eq!(outcome.kwargs.response_cost, Some(chat_price()));
    assert_eq!(outcome.kwargs.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        outcome.kwargs.passthrough_request_body,
        relay.kwargs.passthrough_request_body
    );
    assert_eq!(outcome.kwargs.standard_logging_object, None);
    assert_eq!(relay.call_log.model, "gpt-4o");
    assert_eq!(
        relay.call_log.custom_llm_provider.as_deref(),
        Some("openai")
    );
    assert_eq!(detail(&relay.call_log, "model"), Some(text("gpt-4o")));
    assert_eq!(
        detail(&relay.call_log, "custom_llm_provider"),
        Some(text("openai"))
    );
    assert_eq!(
        detail(&relay.call_log, "response_cost"),
        Some(decimal_json(chat_price()))
    );
    assert!(
        logs.contents()
            .contains("Endpoint: chat_completions, Model: gpt-4o, Cost: $0.001500"),
        "{}",
        logs.contents()
    );
}

#[test]
fn openai_chat_records_the_cost_and_standard_object_calls() {
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, "{}");
    let (tools, _outcome) = run(FakeTools::working(), &mut relay, CHAT);
    let reply = object_json(r#"{"id": "chatcmpl-1", "choices": []}"#);
    assert_eq!(
        tools.calls().get(1..),
        Some(
            &[
                Call::Cost {
                    model: "gpt-4o".to_owned(),
                    provider: "openai".to_owned(),
                    call_type: None,
                    response: reply.clone(),
                },
                Call::Standard {
                    response: reply,
                    logged_cost: Some(decimal_json(chat_price())),
                },
            ][..]
        )
    );
}

#[test]
fn openai_chat_toolbox_decodes_the_upstream_reply() {
    let tools = toolbox(&Arc::new(RecordingPricing::new(Decimal::ONE)), chat_price());
    let body = br#"{"id": "chatcmpl-9", "object": "chat.completion", "created": 1, "model": "gpt-4o", "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}"#;
    let messages = Json::List(Vec::new());
    let params = Json::Object(JsonObject::new());
    let empty = JsonObject::new();
    let mut request = ChatTransformRequest {
        upstream: &UpstreamChatReply {
            status_code: 200,
            body,
        },
        model: "gpt-4o",
        messages: &messages,
        optional_params: &params,
        request_data: &empty,
        json_mode: false,
        litellm_params: &empty,
    };
    let reply = tools.chat_response(&request).unwrap();
    assert_eq!(key(&reply, "id"), &text("chatcmpl-9"));
    let refusal_body = br#"{"id": "chatcmpl-9", "object": "chat.completion", "created": 1, "model": "gpt-4o", "choices": [{"index": 0, "message": {"role": "assistant", "content": null, "refusal": "I cannot help with that"}, "finish_reason": "stop"}]}"#;
    request.upstream = &UpstreamChatReply {
        status_code: 200,
        body: refusal_body,
    };
    let decoded = tools.chat_response(&request).unwrap();
    let Json::List(choices) = key(&decoded, "choices") else {
        panic!("the reply has a choices list");
    };
    let fields = key(key(&choices[0], "message"), "provider_specific_fields");
    assert_eq!(key(fields, "refusal"), &text("I cannot help with that"));
    let broken = UpstreamChatReply {
        status_code: 200,
        body: b"not json",
    };
    request.upstream = &broken;
    assert!(matches!(
        tools.chat_response(&request),
        Err(OpenAiToolsError::Chat(_))
    ));
}

#[test]
fn openai_chat_toolbox_prices_a_chat_reply_with_the_cost_service() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::ONE));
    let tools = toolbox(&pricing, chat_price());
    let reply = object_json(r#"{"id": "chatcmpl-1", "choices": []}"#);
    let cost = tools.completion_cost(&PricedResponse {
        response: &reply,
        model: "gpt-4o",
        custom_llm_provider: "openai",
        call_type: None,
    });
    assert_eq!(cost.unwrap(), chat_price());
    assert!(pricing.calls().is_empty());
}

#[test]
fn openai_embed_response_carries_the_hidden_cost() {
    let mut relay = Relay::new(r#"{"model": "text-embedding-3-small"}"#, EMBEDDING);
    let (tools, outcome) = run(FakeTools::working(), &mut relay, EMBEDDINGS);
    let Some(PassThroughResponse::Embedding(response)) = outcome.result else {
        panic!("an embedding result");
    };
    let hidden = key(&response, "_hidden_params");
    assert_eq!(
        key(hidden, "response_cost"),
        &decimal_json(Decimal::new(2, 3))
    );
    assert_eq!(key(&response, "model"), &text("embed-1"));
    assert_eq!(outcome.kwargs.response_cost, Some(Decimal::new(2, 3)));
    assert_eq!(
        tools.calls().get(..2),
        Some(
            &[
                Call::Embedding,
                Call::EmbeddingCost {
                    model: "text-embedding-3-small".to_owned(),
                    provider: "openai".to_owned(),
                }
            ][..]
        )
    );
}

#[test]
fn openai_embed_decode_failure_returns_the_kwargs_without_a_result() {
    let (logs, _guard) = capture_logs(Level::ERROR);
    let tools = FakeTools {
        embedding: None,
        ..FakeTools::working()
    };
    let mut relay = Relay::new(r#"{"model": "text-embedding-3-small"}"#, EMBEDDING);
    relay.kwargs.litellm_params = object(r#"{"api_base": "x"}"#);
    let (_tools, outcome) = run(tools, &mut relay, EMBEDDINGS);
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, relay.kwargs);
    assert!(
        logs.contents()
            .contains("Error in OpenAI passthrough cost tracking")
    );
}

#[test]
fn openai_embed_toolbox_round_trips_the_embedding_response() {
    let tools = toolbox(&Arc::new(RecordingPricing::new(Decimal::ONE)), chat_price());
    let decoded = tools.embedding_response(&object(EMBEDDING)).unwrap();
    assert_eq!(decoded.get("model"), Some(&text("text-embedding-3-small")));
}

#[rstest]
#[case::absent(
    r#"{"object": "list", "data": [], "model": "m"}"#,
    r#"{"prompt_tokens": 0, "completion_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null, "total_tokens": 0}"#
)]
#[case::null(
    r#"{"object": "list", "data": [], "model": "m", "usage": null}"#,
    r#"{"prompt_tokens": 0, "completion_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null, "total_tokens": 0}"#
)]
#[case::partial(
    r#"{"object": "list", "data": [], "model": "m", "usage": {"total_tokens": 4}}"#,
    r#"{"prompt_tokens": 0, "completion_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null, "total_tokens": 4}"#
)]
fn openai_embed_toolbox_defaults_the_usage_counts(#[case] body: &str, #[case] usage: &str) {
    let tools = toolbox(&Arc::new(RecordingPricing::new(Decimal::ONE)), chat_price());
    let decoded = tools.embedding_response(&object(body)).unwrap();
    assert_eq!(decoded.get("usage"), Some(&object_json(usage)));
}

#[test]
fn openai_embed_absent_usage_still_logs_the_response() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::new(3, 1)));
    let tools = Arc::new(toolbox(&pricing, chat_price()));
    let logger = OpenAiPassthroughLogger::new(tools, fallback_payload());
    let mut relay = Relay::new(
        r#"{"model": "text-embedding-3-small"}"#,
        r#"{"object": "list", "data": [], "model": "m"}"#,
    );
    let outcome = relay.run(&logger, EMBEDDINGS).unwrap();
    let Some(PassThroughResponse::Embedding(response)) = outcome.result else {
        panic!("the embedding reply is logged");
    };
    assert_eq!(
        response.get("usage"),
        Some(&object_json(
            r#"{"prompt_tokens": 0, "completion_tokens": 0, "completion_tokens_details": null, "prompt_tokens_details": null, "total_tokens": 0}"#
        ))
    );
    assert_eq!(outcome.kwargs.response_cost, Some(Decimal::new(3, 1)));
    assert_eq!(
        detail(&relay.call_log, "response_cost"),
        Some(decimal_json(Decimal::new(3, 1)))
    );
}

#[test]
fn openai_embed_toolbox_prices_the_embedding() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::new(3, 1)));
    let tools = toolbox(&pricing, chat_price());
    let cost = tools.embedding_cost(&object(EMBEDDING), "text-embedding-3-small", "openai");
    assert_eq!(cost, Decimal::new(3, 1));
    assert_eq!(
        pricing.calls(),
        vec![PricedCall {
            model: "text-embedding-3-small".to_owned(),
            provider: "openai".to_owned(),
            call_type: CallType::Aembedding,
            response_model: Some("text-embedding-3-small".to_owned()),
        }]
    );
}

#[test]
fn openai_embed_toolbox_bills_zero_when_the_response_does_not_decode() {
    let (logs, _guard) = capture_logs(Level::WARN);
    let pricing = Arc::new(RecordingPricing::new(Decimal::new(3, 1)));
    let tools = toolbox(&pricing, chat_price());
    let cost = tools.embedding_cost(
        &object(r#"{"object": "list"}"#),
        "text-embedding-3-small",
        "openai",
    );
    assert_eq!(cost, Decimal::ZERO);
    assert!(pricing.calls().is_empty());
    assert!(logs.contents().contains("Error calculating embeddings cost for model text-embedding-3-small, logging spend with cost 0"));
}

#[rstest]
#[case::generation(IMAGES, ImageEndpoint::Generation)]
#[case::editing(EDITS, ImageEndpoint::Editing)]
fn openai_image_both_endpoints_build_an_image_reply(
    #[case] url_route: &str,
    #[case] endpoint: ImageEndpoint,
) {
    let request = r#"{"model": "dall-e-3", "quality": "hd"}"#;
    let mut relay = Relay::new(
        request,
        r#"{"created": 1, "model": "dall-e-3", "data": [{"url": "https://images.example/a.png"}]}"#,
    );
    let (tools, outcome) = run(FakeTools::working(), &mut relay, url_route);
    assert_eq!(relay.call_log.call_type, IMAGE_CALL_TYPE);
    assert_eq!(
        tools.calls().first(),
        Some(&Call::ImageCost {
            endpoint,
            model: "dall-e-3".to_owned(),
            request_body: object(request),
        })
    );
    let mut expected = object(
        r#"{"data": [{"url": "https://images.example/a.png"}], "model": "dall-e-3", "usage": {"input_tokens": 0, "input_tokens_details": {"image_tokens": 0, "text_tokens": 0}, "output_tokens": 0, "total_tokens": 0}}"#,
    );
    expected.insert(
        "created".to_owned(),
        Json::Number(Number::from(timestamp(END).timestamp())),
    );
    expected.insert(
        "_hidden_params".to_owned(),
        Json::Object(JsonObject::from([(
            "response_cost".to_owned(),
            decimal_json(Decimal::new(4, 2)),
        )])),
    );
    // IndexMap equality ignores key order, so the built reply is compared as a whole.
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::Image(Json::Object(expected)))
    );
    assert_eq!(outcome.kwargs.response_cost, Some(Decimal::new(4, 2)));
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"data": null}"#)]
fn openai_image_missing_data_gives_an_empty_list(#[case] response: &str) {
    let mut relay = Relay::new(r#"{"model": "dall-e-3"}"#, response);
    let (_tools, outcome) = run(FakeTools::working(), &mut relay, IMAGES);
    let Some(PassThroughResponse::Image(image)) = outcome.result else {
        panic!("an image result");
    };
    assert_eq!(key(&image, "data"), &Json::List(Vec::new()));
}

#[test]
fn openai_image_malformed_data_keeps_the_call_type_and_drops_the_result() {
    let (logs, _guard) = capture_logs(Level::ERROR);
    let payload = fallback_payload();
    let tools = Arc::new(FakeTools::working());
    let mut relay = Relay::new(r#"{"model": "dall-e-3"}"#, r#"{"data": "not-a-list"}"#);
    let outcome = relay.run(&openai_logger(&tools, &payload), EDITS).unwrap();
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, relay.kwargs);
    assert_eq!(relay.call_log.call_type, IMAGE_CALL_TYPE);
    assert!(payload.seen().is_empty());
    assert!(
        logs.contents()
            .contains("Error in OpenAI passthrough cost tracking")
    );
}

#[test]
fn openai_image_toolbox_prices_generation_with_quality_and_editing_without() {
    let tools = toolbox(&Arc::new(RecordingPricing::new(Decimal::ONE)), chat_price());
    let request = object(r#"{"quality": "hd"}"#);
    assert_eq!(
        tools.image_cost(ImageEndpoint::Generation, "dall-e-3", &request),
        Decimal::new(8, 2)
    );
    assert_eq!(
        tools.image_cost(ImageEndpoint::Editing, "dall-e-3", &request),
        Decimal::new(4, 2)
    );
}

#[test]
fn openai_responses_reply_is_built_and_priced_by_the_responses_tool() {
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, "{}");
    let (tools, outcome) = run(FakeTools::working(), &mut relay, RESPONSES);
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ResponsesApi(object_json(
            r#"{"id": "resp-1"}"#
        )))
    );
    assert_eq!(outcome.kwargs.response_cost, Some(Decimal::new(1, 2)));
    assert_eq!(
        tools.calls().first(),
        Some(&Call::Responses {
            model: "gpt-4o".to_owned(),
            provider: "openai".to_owned(),
        })
    );
    assert!(
        !tools
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Cost { .. }))
    );
}

#[test]
fn openai_responses_failure_logs_and_returns_the_kwargs() {
    let payload = fallback_payload();
    let tools = Arc::new(FakeTools {
        responses: None,
        ..FakeTools::working()
    });
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, "{}");
    let outcome = relay
        .run(&openai_logger(&tools, &payload), RESPONSES)
        .unwrap();
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, relay.kwargs);
    assert!(payload.seen().is_empty());
}

#[test]
fn openai_responses_toolbox_returns_the_typed_reply_and_its_cost() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::new(5, 1)));
    let (reply, cost) =
        responses_reply(br#"{"id": "r", "created_at": 5, "output": []}"#, &pricing).unwrap();
    assert_eq!(key(&reply, "id"), &text("r"));
    assert_eq!(cost, Decimal::new(5, 1));
    assert_eq!(
        pricing.calls().first().map(|call| call.call_type),
        Some(CallType::Responses)
    );
}

#[test]
fn openai_responses_toolbox_keeps_a_lax_reply_as_sent() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::new(5, 1)));
    let (reply, _cost) = responses_reply(
        br#"{"id": 7, "created_at": 5, "output": "not-a-list"}"#,
        &pricing,
    )
    .unwrap();
    assert_eq!(key(&reply, "id"), &Json::Number(Number::from(7_i64)));
    assert_eq!(key(&reply, "output"), &text("not-a-list"));
}

#[rstest]
#[case::not_utf8(b"\xff\xfe")]
#[case::not_json(b"{not json")]
fn openai_responses_toolbox_rejects_an_unreadable_body(#[case] body: &[u8]) {
    let pricing = Arc::new(RecordingPricing::new(Decimal::new(5, 1)));
    let error = responses_reply(body, &pricing).unwrap_err();
    assert!(
        matches!(
            error,
            OpenAiToolsError::Text(_) | OpenAiToolsError::Responses(_)
        ),
        "{error}"
    );
    assert!(pricing.calls().is_empty());
}

#[test]
fn openai_responses_toolbox_prices_a_terminal_reply_with_its_call_type() {
    let pricing = Arc::new(RecordingPricing::new(Decimal::new(5, 1)));
    let tools = toolbox(&pricing, chat_price());
    let reply = object_json(r#"{"id": "r", "created_at": 5, "output": []}"#);
    let mut priced = PricedResponse {
        response: &reply,
        model: "gpt-4o",
        custom_llm_provider: "azure",
        call_type: Some(CallType::Responses),
    };
    assert_eq!(tools.completion_cost(&priced).unwrap(), Decimal::new(5, 1));
    assert_eq!(
        pricing.calls(),
        vec![PricedCall {
            model: "gpt-4o".to_owned(),
            provider: "azure".to_owned(),
            call_type: CallType::Responses,
            response_model: None,
        }]
    );
    let broken = object_json(r#"{"id": 5}"#);
    priced.response = &broken;
    assert!(matches!(
        tools.completion_cost(&priced),
        Err(OpenAiToolsError::Encoding(_))
    ));
}

#[test]
fn openai_user_is_merged_into_the_kwargs_and_the_call_log() {
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, EMBEDDING);
    relay.kwargs.passthrough_request_body = Some(object(r#"{"user": "user-1"}"#));
    relay.kwargs.litellm_params = object(
        r#"{"proxy_server_request": {"url": "https://proxy/v1", "body": {"model": "gpt-4o"}}}"#,
    );
    let (_tools, outcome) = run(FakeTools::working(), &mut relay, EMBEDDINGS);
    assert_eq!(
        outcome.kwargs.litellm_params,
        object(
            r#"{"proxy_server_request": {"url": "https://proxy/v1", "body": {"model": "gpt-4o", "user": "user-1"}}}"#
        )
    );
    assert_eq!(
        body_user(&relay.call_log.litellm_params),
        Some(text("user-1"))
    );
}

#[rstest]
#[case::no_body(None)]
#[case::empty(Some(r#"{"user": ""}"#))]
#[case::null(Some(r#"{"user": null}"#))]
#[case::number(Some(r#"{"user": 5}"#))]
#[case::safety_identifier(Some(r#"{"safety_identifier": "s-1"}"#))]
fn openai_user_is_written_only_for_a_non_empty_string(#[case] request_body: Option<&str>) {
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, EMBEDDING);
    relay.kwargs.passthrough_request_body = request_body.map(object);
    let (_tools, outcome) = run(FakeTools::working(), &mut relay, EMBEDDINGS);
    assert!(outcome.kwargs.litellm_params.is_empty());
    assert!(relay.call_log.litellm_params.is_empty());
}

#[rstest]
#[case::null_request(r#"{"proxy_server_request": null}"#)]
#[case::null_body(r#"{"proxy_server_request": {"body": null}}"#)]
fn openai_user_null_entries_become_objects(#[case] litellm_params: &str) {
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, EMBEDDING);
    relay.kwargs.passthrough_request_body = Some(object(r#"{"user": "user-2"}"#));
    relay.kwargs.litellm_params = object(litellm_params);
    let (_tools, outcome) = run(FakeTools::working(), &mut relay, EMBEDDINGS);
    assert_eq!(
        body_user(&outcome.kwargs.litellm_params),
        Some(text("user-2"))
    );
}

#[rstest]
#[case::request(
    r#"{"proxy_server_request": "x"}"#,
    "the litellm_params.proxy_server_request value has the wrong type"
)]
#[case::body(
    r#"{"proxy_server_request": {"body": [1]}}"#,
    "the litellm_params.proxy_server_request.body value has the wrong type"
)]
fn openai_user_entry_of_the_wrong_type_stops_the_logging(
    #[case] litellm_params: &str,
    #[case] message: &str,
) {
    let (logs, _guard) = capture_logs(Level::ERROR);
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, EMBEDDING);
    relay.kwargs.passthrough_request_body = Some(object(r#"{"user": "user-3"}"#));
    relay.kwargs.litellm_params = object(litellm_params);
    let (_tools, outcome) = run(FakeTools::working(), &mut relay, EMBEDDINGS);
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs.response_cost, Some(Decimal::new(2, 3)));
    assert!(logs.contents().contains(message), "{}", logs.contents());
}

#[test]
fn openai_user_payload_uses_the_openai_end_user() {
    let payload = OpenAiPayload {
        cost: Arc::new(FixedCost::new(chat_price())),
        builder: Arc::new(RecordingBuilder::default()),
    };
    assert_eq!(
        payload.end_user(Some(&object(r#"{"user": "user-4"}"#))),
        Some("user-4".to_owned())
    );
    assert_eq!(payload.end_user(Some(&object(r#"{"user": ""}"#))), None);
    assert_eq!(openai_end_user(None), None);
    let mut response = object_json(r#"{"id": "chatcmpl-1"}"#);
    let mut call_log = logging_state();
    let kwargs = payload.build(PassthroughPayloadInput {
        response: &mut response,
        model: "gpt-4o",
        kwargs: ResponseLoggingKwargs {
            response_cost: None,
            model: None,
            litellm_params: JsonObject::new(),
            standard_logging_object: None,
            passthrough_request_body: Some(object(
                r#"{"user": "user-4", "metadata": {"user_id": "other"}}"#,
            )),
        },
        start_time: timestamp(START),
        end_time: timestamp(END),
        call_log: &mut call_log,
    });
    assert_eq!(body_user(&kwargs.litellm_params), Some(text("user-4")));
    assert_eq!(kwargs.response_cost, Some(chat_price()));
}

#[test]
fn openai_fallback_chat_failure_hands_the_kwargs_so_far_to_the_base_handler() {
    let (logs, _guard) = capture_logs(Level::ERROR);
    let payload = fallback_payload();
    let tools = Arc::new(FakeTools::working());
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, "{}");
    relay.kwargs.passthrough_request_body = Some(object(r#"{"user": "user-5"}"#));
    relay.kwargs.litellm_params = object(r#"{"proxy_server_request": "x"}"#);
    let outcome = relay.run(&openai_logger(&tools, &payload), CHAT).unwrap();
    let inputs = payload.inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].response_cost, Some(chat_price()));
    assert_eq!(inputs[0].model.as_deref(), Some("gpt-4o"));
    let Some(PassThroughResponse::ModelResponse(response)) = outcome.result else {
        panic!("the base handler's chat result");
    };
    assert_eq!(key(&response, "payload_model"), &text("gpt-4o"));
    assert!(
        logs.contents()
            .contains("Error in OpenAI passthrough cost tracking")
    );
}

#[test]
fn openai_fallback_cost_failure_falls_back_with_the_kwargs_untouched() {
    let payload = fallback_payload();
    let tools = Arc::new(FakeTools {
        price: None,
        ..FakeTools::working()
    });
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, "{}");
    relay.kwargs.litellm_params = object(r#"{"api_base": "x"}"#);
    let outcome = relay.run(&openai_logger(&tools, &payload), CHAT).unwrap();
    assert_eq!(payload.inputs(), vec![relay.kwargs.clone()]);
    assert_eq!(outcome.kwargs.model.as_deref(), Some("gpt-4o"));
    assert_eq!(payload.seen(), vec!["gpt-4o".to_owned()]);
}

#[test]
fn openai_fallback_wrong_response_format_falls_back_to_the_base_handler() {
    let payload = fallback_payload();
    let tools = Arc::new(FakeTools::working());
    let mut relay = Relay::new(r#"{"model": "gpt-4o", "response_format": "json"}"#, "{}");
    let outcome = relay.run(&openai_logger(&tools, &payload), CHAT).unwrap();
    assert_eq!(payload.seen(), vec!["gpt-4o".to_owned()]);
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(_))
    ));
}

#[test]
fn openai_fallback_error_is_returned_to_the_caller() {
    let tools = Arc::new(FakeTools {
        chat: None,
        ..FakeTools::working()
    });
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, "{}");
    let error = relay
        .run(&openai_logger(&tools, &fallback_payload()), CHAT)
        .unwrap_err();
    assert!(
        matches!(
            error,
            OpenAiPassthroughError::Chat(PassthroughLoggingError::Transform { .. })
        ),
        "{error}"
    );
}

#[rstest]
#[case::images(IMAGES)]
#[case::responses(RESPONSES)]
fn openai_fallback_is_only_for_the_chat_route(#[case] url_route: &str) {
    let payload = fallback_payload();
    let tools = Arc::new(FakeTools {
        price: None,
        responses: None,
        ..FakeTools::working()
    });
    let mut relay = Relay::new(r#"{"model": "gpt-4o"}"#, r#"{"data": 5}"#);
    let outcome = relay
        .run(&openai_logger(&tools, &payload), url_route)
        .unwrap();
    assert_eq!(outcome.result, None);
    assert!(payload.seen().is_empty());
}

#![cfg(test)]

use super::*;

fn payload(text: &str) -> Payload {
    crate::otel::model::metadata::wire::decode::<Payload>(text).expect("payload")
}

fn options(capture_content: bool) -> LLMCallOptions {
    LLMCallOptions {
        capture_content,
        time_to_first_chunk_seconds: None,
        request_route: None,
        trace: TraceControls::default(),
        session_id: None,
    }
}

fn chat_payload() -> Payload {
    payload(
        r#"{
            "id": "call-1",
            "model": "gpt-4o",
            "model_group": "chat-group",
            "api_base": "https://api.openai.com/v1",
            "prompt_tokens": 10,
            "completion_tokens": 4,
            "total_tokens": 14,
            "response_cost": 0.0015,
            "custom_llm_provider": "openai",
            "call_type": "completion",
            "stream": true,
            "model_parameters": {
                "temperature": 0.2,
                "max_tokens": 32,
                "tools": [{"type": "function", "function": {"name": "get_weather", "parameters": {"type": "object"}}}]
            },
            "cost_breakdown": {"input_cost": 0.001, "output_cost": 0.0005},
            "messages": [{"role": "user", "content": "hi"}],
            "response": {
                "id": "resp-1",
                "model": "gpt-4o-2024",
                "system_fingerprint": "fp-1",
                "choices": [{"message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop"}]
            }
        }"#,
    )
}

#[test]
fn span_fields_match_litellm() {
    let span = LLMCallSpanData::from_standard_logging_payload(&chat_payload(), options(false))
        .expect("span");
    assert_eq!(span.operation, GenAIOperation::Chat);
    assert_eq!(span.provider, "openai");
    assert_eq!(span.request_model, "chat-group");
    assert_eq!(span.response_model.as_deref(), Some("gpt-4o-2024"));
    assert_eq!(span.response_id.as_deref(), Some("resp-1"));
    assert_eq!(span.request_params.temperature, Some(0.2));
    assert_eq!(span.request_params.max_tokens, Some(32));
    assert_eq!(span.usage.input_tokens, Some(10));
    assert_eq!(span.usage.output_tokens, Some(4));
    assert_eq!(span.usage.total_tokens, Some(14));
    assert_eq!(span.finish_reasons, vec!["stop".to_owned()]);
    assert_eq!(span.error, None);
    assert_eq!(span.response_cost, Some(Decimal::new(15, 4)));
    assert_eq!(
        span.server,
        Some(ServerInfo {
            address: Some("api.openai.com".to_owned()),
            port: None,
        })
    );
    assert_eq!(span.identity.call_id.as_deref(), Some("call-1"));
    assert_eq!(span.is_streaming, Some(true));
    assert_eq!(span.cost.input, Some(Decimal::new(1, 3)));
    assert_eq!(span.cost.output, Some(Decimal::new(5, 4)));
    assert_eq!(
        span.tools.first().map(|tool| tool.name.as_str()),
        Some("get_weather")
    );
    assert_eq!(span.system_fingerprint.as_deref(), Some("fp-1"));
    assert_eq!(span.time_to_first_chunk_seconds, None);
    assert_eq!(span.output_type, None);
    assert_eq!(span.call_type.as_deref(), Some("completion"));
    assert_eq!(span.request_route, None);
    assert_eq!(span.session_id, None);
    assert_eq!(span.embedding_output, None);
}

#[test]
fn capture_gates_content_only() {
    let hidden = LLMCallSpanData::from_standard_logging_payload(&chat_payload(), options(false))
        .expect("span");
    assert!(hidden.messages_in.is_empty());
    assert!(hidden.choices_out.is_empty());
    assert_eq!(hidden.embedding_output, None);
    assert_eq!(hidden.finish_reasons, vec!["stop".to_owned()]);
    assert_eq!(hidden.tools.len(), 1);

    let shown = LLMCallSpanData::from_standard_logging_payload(&chat_payload(), options(true))
        .expect("span");
    assert_eq!(shown.messages_in.len(), 1);
    assert_eq!(shown.choices_out.len(), 1);
    assert_eq!(shown.finish_reasons, vec!["stop".to_owned()]);
    assert_eq!(shown.tools.len(), 1);
}

#[test]
fn capture_keeps_messages_and_choices() {
    let span = LLMCallSpanData::from_standard_logging_payload(&chat_payload(), options(true))
        .expect("span");
    assert_eq!(
        span.messages_in
            .first()
            .and_then(|message| message.get("role"))
            .and_then(OtelModelValue::as_str),
        Some("user")
    );
    assert_eq!(
        span.choices_out
            .first()
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(OtelModelValue::as_str),
        Some("stop")
    );
}

#[test]
fn call_options_reach_the_span() {
    let span = LLMCallSpanData::from_standard_logging_payload(
        &chat_payload(),
        LLMCallOptions {
            capture_content: false,
            time_to_first_chunk_seconds: Some(Decimal::new(15, 1)),
            request_route: Some("/chat/completions".to_owned()),
            trace: TraceControls {
                name: Some("trace".to_owned()),
                user_id: None,
                session_id: None,
                tags: Vec::new(),
            },
            session_id: Some("session-1".to_owned()),
        },
    )
    .expect("span");
    assert_eq!(span.time_to_first_chunk_seconds, Some(Decimal::new(15, 1)));
    assert_eq!(span.request_route.as_deref(), Some("/chat/completions"));
    assert_eq!(span.session_id.as_deref(), Some("session-1"));
    assert_eq!(span.trace.name.as_deref(), Some("trace"));
}

#[test]
fn empty_options_fall_through_to_the_payload() {
    let raw = payload(
        r#"{
            "model": "gpt-4o",
            "metadata": {"user_api_key_request_route": "/v1/chat/completions"},
            "response": {"choices": []}
        }"#,
    );
    let span = LLMCallSpanData::from_standard_logging_payload(
        &raw,
        LLMCallOptions {
            capture_content: false,
            time_to_first_chunk_seconds: None,
            request_route: Some(String::new()),
            trace: TraceControls::default(),
            session_id: Some(String::new()),
        },
    )
    .expect("span");
    assert_eq!(span.request_route.as_deref(), Some("/v1/chat/completions"));
    assert_eq!(span.session_id, None);
    assert_eq!(span.call_type, None);
    assert_eq!(span.provider, "");
}

#[rstest::rstest]
#[case::float_vectors(
    r#"{"data":[{"object":"embedding","embedding":[0.1,0.2]},{"object":"embedding","embedding":[0.3]}]}"#,
    Some(EmbeddingOutput { count: 2, dimensions: Some(2) })
)]
#[case::base64_rows(
    r#"{"data":[{"embedding":"AAAA"},{"embedding":"BBBB"}]}"#,
    Some(EmbeddingOutput { count: 2, dimensions: None })
)]
#[case::row_without_vector(
    r#"{"data":[{"object":"embedding"}]}"#,
    Some(EmbeddingOutput { count: 1, dimensions: None })
)]
#[case::no_rows(r#"{"data":[]}"#, None)]
#[case::no_data(r#"{"object":"list"}"#, None)]
#[case::first_row_only(
    r#"{"data":[{"embedding":"AAAA"},{"embedding":[1,2,3]}]}"#,
    Some(EmbeddingOutput { count: 2, dimensions: None })
)]
fn embedding_output_matches_litellm(
    #[case] response: &str,
    #[case] expected: Option<EmbeddingOutput>,
) {
    let response: MetadataMap = serde_json::from_str(response).expect("response");
    assert_eq!(EmbeddingOutput::from_response(&response), expected);
}

#[test]
fn embedding_content_follows_the_operation() {
    let raw = payload(
        r#"{"call_type": "embedding", "model_parameters": {"input": "hi"}, "response": {"data": [{"embedding": [1, 2]}]}}"#,
    );
    let span = LLMCallSpanData::from_standard_logging_payload(&raw, options(true)).expect("span");
    assert_eq!(span.operation, GenAIOperation::Embeddings);
    assert_eq!(
        span.embedding_output,
        Some(EmbeddingOutput {
            count: 1,
            dimensions: Some(2)
        })
    );
    let hidden =
        LLMCallSpanData::from_standard_logging_payload(&raw, options(false)).expect("span");
    assert_eq!(hidden.embedding_output, None);
}

#[test]
fn chat_response_with_data_rows_keeps_no_embedding_output() {
    let raw = payload(r#"{"call_type": "completion", "response": {"data": [{"embedding": [1]}]}}"#);
    let span = LLMCallSpanData::from_standard_logging_payload(&raw, options(true)).expect("span");
    assert_eq!(span.embedding_output, None);
}

#[rstest::rstest]
#[case::image("image_generation", Some(GenAIOutputType::Image), None)]
#[case::speech("speech", Some(GenAIOutputType::Speech), None)]
#[case::mcp_tool("call_mcp_tool", None, Some(GenAIOperation::ExecuteTool))]
#[case::unknown("custom_route", None, Some(GenAIOperation::Chat))]
fn call_type_selects_the_operation_and_output_type(
    #[case] call_type: &str,
    #[case] output_type: Option<GenAIOutputType>,
    #[case] operation: Option<GenAIOperation>,
) {
    let raw = payload(&format!(
        r#"{{"call_type": "{call_type}", "response": {{}}}}"#
    ));
    let span = LLMCallSpanData::from_standard_logging_payload(&raw, options(false)).expect("span");
    assert_eq!(span.output_type, output_type);
    if let Some(expected) = operation {
        assert_eq!(span.operation, expected);
    }
    assert_eq!(span.call_type.as_deref(), Some(call_type));
}

#[test]
fn failure_payload_carries_the_span_error() {
    let raw = payload(
        r#"{
            "status": "failure",
            "error_str": "provider exploded",
            "error_information": {"error_class": "APIError", "error_code": "500", "traceback": "trace"},
            "response": {}
        }"#,
    );
    let span = LLMCallSpanData::from_standard_logging_payload(&raw, options(false)).expect("span");
    let error = span.error.expect("span error");
    assert_eq!(error.error_type.as_deref(), Some("APIError"));
    assert_eq!(error.message.as_deref(), Some("provider exploded"));
    assert_eq!(error.code.as_deref(), Some("500"));
    assert_eq!(error.stack_trace.as_deref(), Some("trace"));
}

#[test]
fn text_response_contributes_no_response_fields() {
    let raw = payload(r#"{"response": "plain text"}"#);
    let span = LLMCallSpanData::from_standard_logging_payload(&raw, options(true)).expect("span");
    assert_eq!(span.response_id, None);
    assert_eq!(span.system_fingerprint, None);
    assert!(span.choices_out.is_empty());
}

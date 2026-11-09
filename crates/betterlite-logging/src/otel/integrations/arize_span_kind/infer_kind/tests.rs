#![cfg(test)]

use crate::otel::integrations::arize_span_kind::{
    OpenInferenceSpanKind, infer_open_inference_span_kind,
};

#[rstest::rstest]
#[case::embedding("embedding", OpenInferenceSpanKind::Embedding)]
#[case::async_embedding("aembedding", OpenInferenceSpanKind::Embedding)]
#[case::rerank("rerank", OpenInferenceSpanKind::Reranker)]
#[case::search("search", OpenInferenceSpanKind::Retriever)]
#[case::moderation("moderation", OpenInferenceSpanKind::Guardrail)]
#[case::guardrail("guardrail_check", OpenInferenceSpanKind::Guardrail)]
#[case::mcp_tool("call_mcp_tool", OpenInferenceSpanKind::Tool)]
#[case::mcp_tool_prefix("call_mcp_tool_extra", OpenInferenceSpanKind::Unknown)]
#[case::mcp("mcp", OpenInferenceSpanKind::Tool)]
#[case::tool_suffix("get_weather_tool", OpenInferenceSpanKind::Tool)]
#[case::asend_message("asend_message", OpenInferenceSpanKind::Agent)]
#[case::a2a("a2a_send", OpenInferenceSpanKind::Agent)]
#[case::assistant("assistant_message", OpenInferenceSpanKind::Agent)]
#[case::completion("completion", OpenInferenceSpanKind::Llm)]
#[case::chat("chat", OpenInferenceSpanKind::Llm)]
#[case::image_generation("image_generation", OpenInferenceSpanKind::Llm)]
#[case::audio_speech("audio_speech", OpenInferenceSpanKind::Llm)]
#[case::speech("speech", OpenInferenceSpanKind::Llm)]
#[case::case_insensitive("Anthropic_Messages", OpenInferenceSpanKind::Llm)]
#[case::pass_through("pass_through_route", OpenInferenceSpanKind::Llm)]
#[case::passthrough("allm_passthrough_route", OpenInferenceSpanKind::Llm)]
#[case::ocr("ocr", OpenInferenceSpanKind::Llm)]
#[case::realtime("realtime", OpenInferenceSpanKind::Llm)]
#[case::videos("videos", OpenInferenceSpanKind::Llm)]
#[case::generate_content("generate_content", OpenInferenceSpanKind::Llm)]
#[case::batch("batch", OpenInferenceSpanKind::Chain)]
#[case::file("file", OpenInferenceSpanKind::Chain)]
#[case::fine_tuning_job("fine_tuning_job", OpenInferenceSpanKind::Chain)]
#[case::container("container", OpenInferenceSpanKind::Chain)]
#[case::unknown("something_else", OpenInferenceSpanKind::Unknown)]
fn infer_kind_maps_the_call_type(#[case] call_type: &str, #[case] expected: OpenInferenceSpanKind) {
    assert_eq!(infer_open_inference_span_kind(Some(call_type)), expected);
}

#[test]
fn infer_kind_returns_unknown_without_a_call_type() {
    assert_eq!(
        infer_open_inference_span_kind(None),
        OpenInferenceSpanKind::Unknown
    );
    assert_eq!(
        infer_open_inference_span_kind(Some("")),
        OpenInferenceSpanKind::Unknown
    );
}

#[test]
fn infer_kind_checks_the_keyword_families_in_order() {
    assert_eq!(
        infer_open_inference_span_kind(Some("embedding_search")),
        OpenInferenceSpanKind::Embedding
    );
    assert_eq!(
        infer_open_inference_span_kind(Some("batch_embedding")),
        OpenInferenceSpanKind::Embedding
    );
    assert_eq!(
        infer_open_inference_span_kind(Some("fine_tuning")),
        OpenInferenceSpanKind::Unknown
    );
}

#[test]
fn infer_kind_names_the_open_inference_span_kinds() {
    assert_eq!(OpenInferenceSpanKind::Llm.to_string(), "LLM");
    assert_eq!(OpenInferenceSpanKind::Unknown.to_string(), "UNKNOWN");
    assert_eq!(OpenInferenceSpanKind::Guardrail.to_string(), "GUARDRAIL");
}

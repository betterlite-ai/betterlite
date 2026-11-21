#![cfg(test)]

use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_translation::BaseTranslation;

use crate::guardrails::unified_guardrail::other_adapters::RerankTranslation;
use crate::guardrails::unified_guardrail::other_adapters::google_genai::GoogleGenAiTranslation;
use crate::guardrails::unified_guardrail::other_adapters::mcp::McpTranslation;
use crate::guardrails::unified_guardrail::tests::support::{
    FakeGuardrail, blocked, no_call, object, value,
};

fn masked() -> GuardrailValue {
    GuardrailValue::Text(String::from("[masked]"))
}

#[tokio::test]
async fn rerank_masks_the_query_and_leaves_the_scores() {
    let guardrail = FakeGuardrail::masking();
    let mut data = object(r#"{"model": "rerank-v3", "query": "secret", "documents": ["a"]}"#);
    let rerank = RerankTranslation(PhantomData);
    rerank
        .process_input_messages(&mut data, guardrail.as_ref(), None)
        .await
        .unwrap();
    assert_eq!(data.get("query"), Some(&masked()));
    assert_eq!(
        guardrail.seen()[0].inputs.model.as_deref(),
        Some("rerank-v3")
    );
    let response = value(r#"{"results": [{"index": 0, "relevance_score": 0.5}]}"#);
    let checked = rerank
        .process_output_response(response.clone(), guardrail.as_ref(), no_call())
        .await
        .unwrap();
    assert_eq!(checked, response);
    assert_eq!(guardrail.seen().len(), 1);
}

#[tokio::test]
async fn rerank_failure_is_the_guardrail_failure() {
    let guardrail = FakeGuardrail::failing(blocked());
    let mut data = object(r#"{"query": "secret"}"#);
    let failure = RerankTranslation(PhantomData)
        .process_input_messages(&mut data, guardrail.as_ref(), None)
        .await
        .unwrap_err();
    assert_eq!(failure, blocked());
}

#[test]
fn adapters_keep_the_trait_defaults() {
    let data = object(r#"{"tools": [{"type": "function", "function": {"name": "lookup"}}]}"#);
    let response = value(r#"{"results": []}"#);
    let rerank: RerankTranslation<FakeGuardrail> = RerankTranslation(PhantomData);
    let genai: GoogleGenAiTranslation<FakeGuardrail> = GoogleGenAiTranslation(PhantomData);
    let mcp: McpTranslation<FakeGuardrail> = McpTranslation(PhantomData);
    assert!(rerank.extract_request_tool_names(&data).is_empty());
    assert!(genai.get_structured_messages(&data).is_none());
    assert!(mcp.get_streaming_scan_key(&[]).is_none());
    assert!(mcp.build_block_sse_chunks(&blocked(), true, None).is_none());
    assert!(genai.build_stream_error_items(&blocked(), None).is_none());
    assert_eq!(rerank.post_call_hook_response(response.clone()), response);
    assert!(!<McpTranslation<FakeGuardrail> as BaseTranslation>::DELIVERS_ENDED_STREAM_REWRITES);
    assert!(
        !<GoogleGenAiTranslation<FakeGuardrail> as BaseTranslation>::ASSEMBLES_STREAMED_RESPONSE
    );
}

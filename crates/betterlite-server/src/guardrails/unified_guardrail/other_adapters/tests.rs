#![cfg(test)]

use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};

use crate::guardrails::unified_guardrail::other_adapters::{
    GoogleGenAiTranslation, McpTranslation, RerankTranslation,
};
use crate::guardrails::unified_guardrail::tests::support::{FakeGuardrail, object, value};

fn no_call<'call>() -> TranslationCall<'call, GuardrailLogTarget> {
    TranslationCall {
        user_api_key_dict: None,
        request_data: None,
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    }
}

fn blocked() -> GuardrailFailure {
    GuardrailFailure::Http {
        status: 400,
        detail: String::from("blocked"),
    }
}

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

#[tokio::test]
async fn google_genai_masks_request_and_response_text() {
    let guardrail = FakeGuardrail::masking();
    let genai = GoogleGenAiTranslation(PhantomData);
    let mut data = object(
        r#"{"model": "gemini", "contents": [{"role": "user", "parts": [{"text": "secret"}]}]}"#,
    );
    genai
        .process_input_messages(&mut data, guardrail.as_ref(), None)
        .await
        .unwrap();
    assert_eq!(
        GuardrailValue::Object(data),
        value(
            r#"{"model": "gemini", "contents": [{"role": "user", "parts": [{"text": "[masked]"}]}]}"#
        )
    );
    let response = value(r#"{"candidates": [{"content": {"parts": [{"text": "reply"}]}}]}"#);
    let checked = genai
        .process_output_response(response, guardrail.as_ref(), no_call())
        .await
        .unwrap();
    assert_eq!(
        checked,
        value(r#"{"candidates": [{"content": {"parts": [{"text": "[masked]"}]}}]}"#)
    );
    let seen = guardrail.seen();
    assert_eq!(seen[0].input_type, GuardrailInputType::Request);
    assert_eq!(seen[1].input_type, GuardrailInputType::Response);
    assert_eq!(seen[1].inputs.texts, Some(vec![String::from("reply")]));
}

#[tokio::test]
async fn google_genai_scans_the_stream_text_so_far() {
    let guardrail = FakeGuardrail::masking();
    let mut frames = vec![
        value(r#"{"candidates": [{"content": {"parts": [{"text": "sec"}]}}]}"#),
        value(r#"{"candidates": [{"content": {"parts": [{"text": "ret"}]}}]}"#),
    ];
    let before = frames.clone();
    GoogleGenAiTranslation(PhantomData)
        .process_streaming_response(&mut frames, guardrail.as_ref(), no_call())
        .await
        .unwrap();
    assert_eq!(frames, before);
    assert_eq!(guardrail.seen_texts(), vec![vec![String::from("secret")]]);
}

#[tokio::test]
async fn google_genai_leaves_a_response_that_is_not_an_object() {
    let guardrail = FakeGuardrail::masking();
    let response = value(r#"["not", "an", "object"]"#);
    let checked = GoogleGenAiTranslation(PhantomData)
        .process_output_response(response.clone(), guardrail.as_ref(), no_call())
        .await
        .unwrap();
    assert_eq!(checked, response);
    assert!(guardrail.seen().is_empty());
}

#[tokio::test]
async fn mcp_masks_tool_arguments_and_sends_the_tool() {
    let guardrail = FakeGuardrail::masking();
    let mut data = object(r#"{"mcp_tool_name": "lookup", "mcp_arguments": {"query": "secret"}}"#);
    McpTranslation(PhantomData)
        .process_input_messages(&mut data, guardrail.as_ref(), None)
        .await
        .unwrap();
    let arguments = value(r#"{"query": "[masked]"}"#);
    assert_eq!(data.get("mcp_arguments"), Some(&arguments));
    assert_eq!(data.get("modified_arguments"), Some(&arguments));
    let seen = guardrail.seen();
    assert_eq!(seen[0].inputs.texts, Some(vec![String::from("secret")]));
    assert_eq!(seen[0].inputs.tools.as_ref().map(Vec::len), Some(1));
}

#[tokio::test]
async fn mcp_masks_the_tool_result_text() {
    let guardrail = FakeGuardrail::masking();
    let response = value(r#"{"content": [{"type": "text", "text": "secret"}]}"#);
    let mut request = object(r#"{"mcp_tool_name": "lookup"}"#);
    let call = TranslationCall {
        user_api_key_dict: None,
        request_data: Some(&mut request),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    let checked = McpTranslation(PhantomData)
        .process_output_response(response, guardrail.as_ref(), call)
        .await
        .unwrap();
    assert_eq!(
        checked,
        value(r#"{"content": [{"type": "text", "text": "[masked]"}]}"#)
    );
    assert_eq!(guardrail.seen()[0].input_type, GuardrailInputType::Response);
}

#[tokio::test]
async fn mcp_failure_is_the_guardrail_failure() {
    let guardrail = FakeGuardrail::failing(blocked());
    let mut data = object(r#"{"mcp_tool_name": "lookup", "mcp_arguments": {"query": "secret"}}"#);
    let failure = McpTranslation(PhantomData)
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

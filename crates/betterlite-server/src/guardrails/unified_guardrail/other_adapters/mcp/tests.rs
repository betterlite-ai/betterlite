#![cfg(test)]

use std::marker::PhantomData;

use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_translation::{BaseTranslation as _, TranslationCall};

use crate::guardrails::unified_guardrail::other_adapters::mcp::McpTranslation;
use crate::guardrails::unified_guardrail::tests::support::{FakeGuardrail, blocked, object, value};

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

#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::mcp_guardrail_translation::error::TranslationError;
use crate::mcp_guardrail_translation::process_input::process_input_messages;
use crate::mcp_guardrail_translation::tests::{Reply, ScriptedGuardrail, request, texts, to_json};

#[tokio::test]
async fn sends_description_schema_descriptions_and_argument_leaves_in_one_call() {
    let mut call = request(
        r#"{"mcp_tool_name":"lookup","mcp_tool_description":"Find a user",
            "mcp_input_schema":{"type":"object","properties":{"email":{"type":"string","description":"The email"}}},
            "mcp_arguments":{"email":"a@example.com","filters":[{"tag":"vip"},7,true]}}"#,
    );
    let guardrail = ScriptedGuardrail::new(Reply::Nothing);
    process_input_messages(&mut call, &guardrail).await.unwrap();
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    let (inputs, input_type) = &calls[0];
    assert_eq!(*input_type, GuardrailInputType::Request);
    assert_eq!(
        inputs.texts,
        texts(&["Find a user", "The email", "a@example.com", "vip"])
    );
    assert_eq!(inputs.tools.len(), 1);
    assert_eq!(
        to_json(&inputs.tools[0]),
        r#"{"type":"function","function":{"description":"Find a user","name":"lookup","parameters":{"additionalProperties":false,"properties":{"email":{"description":"The email","type":"string"}},"type":"object"},"strict":false}}"#
    );
}

#[tokio::test]
async fn an_empty_schema_becomes_an_empty_object_definition() {
    let mut call = request(r#"{"mcp_tool_name":"ping","mcp_input_schema":"not a schema"}"#);
    let guardrail = ScriptedGuardrail::new(Reply::Nothing);
    process_input_messages(&mut call, &guardrail).await.unwrap();
    let calls = guardrail.calls();
    let (inputs, _) = &calls[0];
    assert!(inputs.texts.is_empty());
    assert_eq!(
        inputs.tools[0].extra.get("function"),
        Some(
            &serde_json::from_str::<GuardrailValue>(
                r#"{"name":"ping","description":"","strict":false,
                "parameters":{"type":"object","properties":{},"additionalProperties":false}}"#
            )
            .unwrap()
        )
    );
}

#[tokio::test]
async fn only_description_keyed_schema_strings_are_scanned() {
    let mut call = request(
        r#"{"mcp_tool_name":"lookup","mcp_input_schema":{"title":"Lookup","description":"Top",
            "properties":{"description":{"type":"string","enum":["a"]}}}}"#,
    );
    let guardrail = ScriptedGuardrail::new(Reply::Nothing);
    process_input_messages(&mut call, &guardrail).await.unwrap();
    assert_eq!(guardrail.calls()[0].0.texts, texts(&["Top"]));
}

#[tokio::test]
async fn a_guardrail_failure_reaches_the_caller() {
    let mut call = request(r#"{"mcp_tool_name":"lookup","mcp_arguments":{"q":"x"}}"#);
    let guardrail = ScriptedGuardrail::new(Reply::Fail);
    let outcome = process_input_messages(&mut call, &guardrail).await;
    assert!(matches!(outcome, Err(TranslationError::Guardrail(_))));
}

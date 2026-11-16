#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use rstest::rstest;

use super::{InputScope, process_input_messages};
use crate::guardrails::chat_guardrail_translation::extraction::not_run_reason::{
    AFTER_SCOPING, NOTHING_SCANNABLE,
};
use crate::guardrails::chat_guardrail_translation::rewrites::error::{InputError, RewriteError};
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::{
    FakeGuardrail, object, values,
};

const REQUEST: &str = r#"{"model": "gpt-4o", "messages": [
    {"role": "system", "content": "be nice"},
    {"role": "user", "content": [{"type": "text", "text": "look"}, {"type": "image_url", "image_url": {"url": "https://i/1.png"}}]},
    {"role": "assistant", "content": null, "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
    {"role": "user", "content": "bye"}],
    "tools": [{"type": "function", "function": {"name": "f"}}]}"#;

const TOOL_RESULT_REQUEST: &str = r#"{"messages": [
    {"role": "user", "content": "hi"}, {"role": "tool", "content": "result"}],
    "tools": [{"type": "function", "function": {"name": "f"}}]}"#;

const ALL: ScopeFlags = ScopeFlags {
    skip_system_message: false,
    skip_tool_message: false,
    scan: ScanScope::AllMessages,
};

const WITHOUT_SYSTEM: ScopeFlags = ScopeFlags {
    skip_system_message: true,
    skip_tool_message: false,
    scan: ScanScope::AllMessages,
};

const TOOL_RESULTS_ONLY: ScopeFlags = ScopeFlags {
    skip_system_message: false,
    skip_tool_message: false,
    scan: ScanScope::ToolResultsOnly,
};

async fn run(
    source: &str,
    guardrail: &FakeGuardrail,
    flags: ScopeFlags,
) -> (Result<Option<&'static str>, InputError>, Object) {
    let mut data = object(source);
    let scope = InputScope {
        flags,
        guardrail_name: Some("pii"),
    };
    let result = process_input_messages(&mut data, guardrail, &scope, None).await;
    (result, data)
}

fn upper_cased(inputs: GuardrailApiInputs) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: inputs
            .texts
            .map(|texts| texts.iter().map(|text| text.to_uppercase()).collect()),
        ..inputs
    }
}

fn redacted(message: GuardrailMessage) -> GuardrailMessage {
    let mut extra = message.extra;
    extra.insert(
        "content".to_owned(),
        GuardrailValue::Text("REDACTED".to_owned()),
    );
    GuardrailMessage {
        role: message.role,
        extra,
    }
}

fn tool(name: &str) -> GuardrailToolParam {
    let function = GuardrailValue::Object(object(&format!(r#"{{"name": "{name}"}}"#)));
    GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: Object::from([("function".to_owned(), function)]),
    }
}

#[tokio::test]
async fn input_messages_send_the_extracted_content_as_a_request_input() {
    let guardrail = FakeGuardrail::passing_through();
    let (result, data) = run(REQUEST, &guardrail, ALL).await;
    assert_eq!(result.unwrap(), None);
    assert_eq!(data, object(REQUEST));
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.input_type, GuardrailInputType::Request);
    assert_eq!(
        call.inputs.texts,
        Some(vec![
            "be nice".to_owned(),
            "look".to_owned(),
            "bye".to_owned()
        ])
    );
    assert_eq!(call.inputs.images, Some(vec!["https://i/1.png".to_owned()]));
    assert_eq!(call.inputs.tool_calls.as_ref().map(Vec::len), Some(1));
    assert_eq!(call.inputs.tools, Some(vec![tool("f")]));
    assert_eq!(call.inputs.model.as_deref(), Some("gpt-4o"));
    let roles: Vec<&str> = call
        .inputs
        .structured_messages
        .iter()
        .flatten()
        .map(|message| message.role.as_str())
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
}

#[tokio::test]
async fn input_messages_leave_out_the_parts_the_request_does_not_have() {
    let guardrail = FakeGuardrail::passing_through();
    let source = r#"{"model": "", "messages": [{"role": "user", "content": "hi"}], "tools": []}"#;
    let (result, _data) = run(source, &guardrail, ALL).await;
    result.unwrap();
    let inputs = &guardrail.calls()[0].inputs;
    assert_eq!(inputs.images, None);
    assert_eq!(inputs.tool_calls, None);
    assert_eq!(inputs.tools, None);
    assert_eq!(inputs.model, None);
}

#[tokio::test]
async fn input_messages_send_only_the_scoped_messages_as_structured_messages() {
    let guardrail = FakeGuardrail::passing_through();
    let (result, _data) = run(REQUEST, &guardrail, WITHOUT_SYSTEM).await;
    result.unwrap();
    let inputs = &guardrail.calls()[0].inputs;
    assert_eq!(
        inputs.texts,
        Some(vec!["look".to_owned(), "bye".to_owned()])
    );
    assert_eq!(inputs.structured_messages.as_ref().map(Vec::len), Some(3));
}

#[tokio::test]
async fn input_messages_do_not_send_tools_when_only_tool_results_are_scanned() {
    let guardrail = FakeGuardrail::passing_through();
    let (result, _data) = run(TOOL_RESULT_REQUEST, &guardrail, TOOL_RESULTS_ONLY).await;
    result.unwrap();
    let inputs = &guardrail.calls()[0].inputs;
    assert_eq!(inputs.tools, None);
    assert_eq!(inputs.texts, Some(vec!["result".to_owned()]));
}

#[tokio::test]
async fn input_messages_rewrite_string_and_list_text_in_place() {
    let guardrail = FakeGuardrail::answering(Box::new(upper_cased));
    let (result, data) = run(REQUEST, &guardrail, ALL).await;
    result.unwrap();
    let expected = REQUEST
        .replace(r#""be nice""#, r#""BE NICE""#)
        .replace(r#""text": "look""#, r#""text": "LOOK""#)
        .replace(r#""bye""#, r#""BYE""#);
    assert_eq!(data, object(&expected));
}

#[tokio::test]
async fn input_messages_map_texts_back_by_the_full_message_index_when_system_is_skipped() {
    let guardrail = FakeGuardrail::answering(Box::new(upper_cased));
    let (result, data) = run(REQUEST, &guardrail, WITHOUT_SYSTEM).await;
    result.unwrap();
    let expected = REQUEST
        .replace(r#""text": "look""#, r#""text": "LOOK""#)
        .replace(r#""bye""#, r#""BYE""#);
    assert_eq!(data, object(&expected));
}

#[tokio::test]
async fn input_messages_replace_tool_calls_by_message_and_call_index() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        tool_calls: Some(vec![GuardrailValue::Object(object(
            r#"{"id": "c1", "function": {"name": "g"}}"#,
        ))]),
        ..inputs
    }));
    let (result, data) = run(REQUEST, &guardrail, ALL).await;
    result.unwrap();
    let expected = REQUEST.replace(
        r#"{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}"#,
        r#"{"id": "c1", "function": {"name": "g"}}"#,
    );
    assert_eq!(data, object(&expected));
}

#[tokio::test]
async fn input_messages_refuse_a_different_number_of_texts() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        texts: Some(vec!["only".to_owned()]),
        ..inputs
    }));
    let (result, data) = run(REQUEST, &guardrail, ALL).await;
    assert!(matches!(
        result.unwrap_err(),
        InputError::Unappliable(rewrite) if rewrite.guardrail_name == "pii"
    ));
    assert_eq!(data, object(REQUEST));
}

#[tokio::test]
async fn input_messages_replace_the_whole_list_when_the_guardrail_covers_the_full_request() {
    let reply = Box::new(|inputs: GuardrailApiInputs| GuardrailApiInputs {
        structured_messages: inputs
            .structured_messages
            .map(|messages| messages.into_iter().map(redacted).collect()),
        ..inputs
    });
    let guardrail = FakeGuardrail::answering(reply).covering_full_request();
    let (result, data) = run(REQUEST, &guardrail, WITHOUT_SYSTEM).await;
    result.unwrap();
    let expected = r#"{"model": "gpt-4o", "messages": [
    {"role": "user", "content": "REDACTED"},
    {"role": "assistant", "content": "REDACTED", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
    {"role": "user", "content": "REDACTED"}],
    "tools": [{"type": "function", "function": {"name": "f"}}]}"#;
    assert_eq!(data, object(expected));
}

#[tokio::test]
async fn input_messages_merge_scoped_messages_back_when_the_guardrail_saw_only_part() {
    let reply = Box::new(|inputs: GuardrailApiInputs| GuardrailApiInputs {
        structured_messages: inputs
            .structured_messages
            .map(|messages| messages.into_iter().map(redacted).collect()),
        ..inputs
    });
    let guardrail = FakeGuardrail::answering(reply);
    let (result, data) = run(REQUEST, &guardrail, WITHOUT_SYSTEM).await;
    result.unwrap();
    let Some(GuardrailValue::Array(messages)) = data.get("messages") else {
        panic!("messages missing");
    };
    let expected = r#"[
    {"role": "system", "content": "be nice"},
    {"role": "user", "content": "REDACTED"},
    {"role": "assistant", "content": "REDACTED", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]},
    {"role": "user", "content": "REDACTED"}]"#;
    assert_eq!(
        GuardrailValue::Array(messages.clone()),
        GuardrailValue::Array(values(expected))
    );
}

#[tokio::test]
async fn input_messages_replace_the_request_tools_with_the_returned_ones() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        tools: Some(vec![tool("other")]),
        ..inputs
    }));
    let (result, data) = run(REQUEST, &guardrail, ALL).await;
    result.unwrap();
    let expected = REQUEST.replace(r#""name": "f"}}]}"#, r#""name": "other"}}]}"#);
    assert_eq!(data, object(&expected));
}

#[tokio::test]
async fn input_messages_add_only_new_tools_when_only_tool_results_are_scanned() {
    let guardrail = FakeGuardrail::answering(Box::new(|inputs| GuardrailApiInputs {
        tools: Some(vec![tool("extra"), tool("f")]),
        ..inputs
    }));
    let (result, data) = run(TOOL_RESULT_REQUEST, &guardrail, TOOL_RESULTS_ONLY).await;
    result.unwrap();
    let expected = TOOL_RESULT_REQUEST.replace(
        r#""tools": [{"type": "function", "function": {"name": "f"}}]"#,
        r#""tools": [{"type": "function", "function": {"name": "f"}}, {"type": "function", "function": {"name": "extra"}}]"#,
    );
    assert_eq!(data, object(&expected));
}

#[tokio::test]
async fn input_messages_pass_a_guardrail_failure_on() {
    let blocked = GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    };
    let guardrail = FakeGuardrail::failing(blocked.clone());
    let (result, data) = run(REQUEST, &guardrail, ALL).await;
    assert!(matches!(result.unwrap_err(), InputError::Guardrail(failure) if failure == blocked));
    assert_eq!(data, object(REQUEST));
}

#[rstest]
#[case::no_key(r#"{"model": "m"}"#)]
#[case::null(r#"{"messages": null}"#)]
#[tokio::test]
async fn input_messages_do_nothing_without_messages(#[case] source: &str) {
    let guardrail = FakeGuardrail::passing_through();
    let (result, _data) = run(source, &guardrail, ALL).await;
    assert_eq!(result.unwrap(), None);
    assert!(guardrail.calls().is_empty());
}

#[tokio::test]
async fn input_messages_refuse_messages_that_are_not_a_list() {
    let guardrail = FakeGuardrail::passing_through();
    let (result, _data) = run(r#"{"messages": "hi"}"#, &guardrail, ALL).await;
    assert!(matches!(
        result.unwrap_err(),
        InputError::Rewrite(RewriteError::NotAList { field: "messages" })
    ));
}

#[rstest]
#[case::empty_content(
    r#"{"messages": [{"role": "user", "content": null}]}"#,
    ALL,
    Some(NOTHING_SCANNABLE)
)]
#[case::scoped_out(
    r#"{"messages": [{"role": "system", "content": "rules"}]}"#,
    WITHOUT_SYSTEM,
    Some(AFTER_SCOPING)
)]
#[case::images_only(
    r#"{"messages": [{"role": "user", "content": [{"type": "image_url", "image_url": "https://i/1.png"}]}]}"#,
    ALL,
    None
)]
#[tokio::test]
async fn input_messages_report_why_nothing_was_scanned(
    #[case] source: &str,
    #[case] flags: ScopeFlags,
    #[case] expected: Option<&str>,
) {
    let guardrail = FakeGuardrail::passing_through();
    let (result, _data) = run(source, &guardrail, flags).await;
    assert_eq!(result.unwrap(), expected);
    assert!(guardrail.calls().is_empty());
}

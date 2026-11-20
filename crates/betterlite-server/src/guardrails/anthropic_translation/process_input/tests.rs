#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    GlobalSkipFlags, GuardrailSkipFlags,
};
use serde_json::{Value, json};

use super::error::ProcessInputError;
use super::{InputContext, process_input_messages};

pub(crate) const NO_INPUTS: GuardrailApiInputs = GuardrailApiInputs {
    texts: None,
    images: None,
    tools: None,
    tool_calls: None,
    structured_messages: None,
    model: None,
    stream_holdback_chars: None,
};

const NO_SKIPS: GuardrailSkipFlags = GuardrailSkipFlags {
    skip_system_message: None,
    skip_tool_message: None,
    scan_only_tool_results: None,
};

#[derive(Clone, Debug)]
pub(crate) struct Recording {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
}

pub(crate) struct ScriptedGuardrail {
    answer: GuardrailApiInputs,
    covers_full_request: bool,
    recorded: Mutex<Option<Recording>>,
}

impl ScriptedGuardrail {
    pub(crate) fn new(answer: GuardrailApiInputs) -> Self {
        Self {
            answer,
            covers_full_request: false,
            recorded: Mutex::new(None),
        }
    }

    pub(crate) fn covering_full_request(mut self) -> Self {
        self.covers_full_request = true;
        self
    }

    pub(crate) fn ran(&self) -> bool {
        self.recorded
            .lock()
            .expect("the recording lock is not poisoned")
            .is_some()
    }

    pub(crate) fn recording(&self) -> Recording {
        self.recorded
            .lock()
            .expect("the recording lock is not poisoned")
            .clone()
            .expect("the guardrail recorded its call")
    }
}

impl GuardrailContract for ScriptedGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        let recording = Recording {
            inputs,
            input_type,
            request_data: request_data.clone(),
        };
        *self
            .recorded
            .lock()
            .expect("the recording lock is not poisoned") = Some(recording);
        let answer = self.answer.clone();
        async move { Ok(answer) }
    }

    fn messages_cover_full_request(&self) -> bool {
        self.covers_full_request
    }
}

fn body(value: Value) -> Object {
    serde_json::from_value(value).expect("a request body")
}

fn to_json(value: &GuardrailValue) -> Value {
    serde_json::to_value(value).expect("a JSON value")
}

fn message_json(data: &Object, msg_idx: usize) -> Value {
    let messages = data.get("messages").expect("the request messages");
    let array = to_json(messages);
    array.as_array().expect("an array")[msg_idx].clone()
}

fn context() -> InputContext<'static> {
    context_with(NO_SKIPS, GlobalSkipFlags::default())
}

fn context_with(
    skip_flags: GuardrailSkipFlags,
    global_flags: GlobalSkipFlags,
) -> InputContext<'static> {
    InputContext {
        skip_flags,
        global_flags,
        guardrail_name: Some("test-guardrail"),
    }
}

fn row(role: &str, content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        )]),
    }
}

fn returned_tool(name: &str) -> GuardrailToolParam {
    serde_json::from_value(json!({
        "type": "function",
        "function": {
            "name": name,
            "parameters": {"type": "object", "properties": {}},
            "description": "n"
        }
    }))
    .expect("a chat tool")
}

fn tool_exchange_body() -> Object {
    body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [
            {"role": "system", "content": "mid"},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "res"}]},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "get", "input": {"a": 1}}]},
            {"role": "user", "content": "next"}
        ]
    }))
}

fn roles(rows: &[GuardrailMessage]) -> Vec<&str> {
    rows.iter().map(|row| row.role.as_str()).collect()
}

#[tokio::test]
async fn inputs_carry_texts_images_and_model() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": [
            {"type": "text", "text": "look"},
            {"type": "image", "source": {"type": "url", "url": "http://x/y.png"}}
        ]}]
    }));
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    let recording = guardrail.recording();
    assert_eq!(recording.input_type, GuardrailInputType::Request);
    assert_eq!(recording.inputs.texts, Some(vec!["look".to_owned()]));
    assert_eq!(
        recording.inputs.images,
        Some(vec!["http://x/y.png".to_owned()])
    );
    assert_eq!(recording.inputs.model, Some("claude-sonnet-4-5".to_owned()));
    assert!(recording.inputs.tool_calls.is_none());
    assert!(recording.inputs.tools.is_none());
    let rows = recording
        .inputs
        .structured_messages
        .expect("structured rows");
    assert_eq!(roles(&rows), vec!["user"]);
}

#[tokio::test]
async fn inputs_are_not_built_when_nothing_is_scannable() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": [
            {"type": "image", "source": {"type": "url", "url": "http://x/y.png"}}
        ]}]
    }));
    let before = data.clone();
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    assert!(!guardrail.ran());
    assert_eq!(data, before);
}

#[tokio::test]
async fn inputs_put_the_top_level_system_text_first() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "system": "trusted",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    let recording = guardrail.recording();
    assert_eq!(
        recording.inputs.texts,
        Some(vec!["trusted".to_owned(), "hi".to_owned()])
    );
    let rows = recording
        .inputs
        .structured_messages
        .expect("structured rows");
    assert_eq!(roles(&rows), vec!["system", "user"]);
}

#[tokio::test]
async fn flags_unset_system_skip_falls_back_to_the_global_flag() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "system": "trusted",
        "messages": [
            {"role": "system", "content": "mid"},
            {"role": "user", "content": "hi"}
        ]
    }));
    let global = GlobalSkipFlags {
        skip_system_message: true,
        skip_tool_message: false,
    };
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_input_messages(&mut data, &guardrail, &context_with(NO_SKIPS, global), None)
        .await
        .unwrap();
    let recording = guardrail.recording();
    assert_eq!(
        recording.inputs.texts,
        Some(vec!["mid".to_owned(), "hi".to_owned()])
    );
    let rows = recording
        .inputs
        .structured_messages
        .expect("structured rows");
    assert_eq!(roles(&rows), vec!["system", "user"]);
    assert_eq!(
        to_json(data.get("system").expect("the system prompt")),
        json!("trusted")
    );
}

#[tokio::test]
async fn flags_guardrail_system_skip_wins_over_a_global_scan() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "system": "trusted",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let skip_system = GuardrailSkipFlags {
        skip_system_message: Some(true),
        ..NO_SKIPS
    };
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_input_messages(
        &mut data,
        &guardrail,
        &context_with(skip_system, GlobalSkipFlags::default()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        guardrail.recording().inputs.texts,
        Some(vec!["hi".to_owned()])
    );
}

#[tokio::test]
async fn scope_skip_tool_drops_the_tool_row_and_its_texts() {
    let mut data = tool_exchange_body();
    let skip_tool = GuardrailSkipFlags {
        skip_tool_message: Some(true),
        ..NO_SKIPS
    };
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_input_messages(
        &mut data,
        &guardrail,
        &context_with(skip_tool, GlobalSkipFlags::default()),
        None,
    )
    .await
    .unwrap();
    let recording = guardrail.recording();
    // The tool result text goes, but the mid-turn system row and the plain user text stay in the scan.
    assert_eq!(
        recording.inputs.texts,
        Some(vec!["mid".to_owned(), "next".to_owned()])
    );
    let rows = recording
        .inputs
        .structured_messages
        .expect("structured rows");
    assert_eq!(roles(&rows), vec!["system", "assistant", "user"]);
    assert_eq!(recording.inputs.tool_calls.expect("tool calls").len(), 1);
}

#[tokio::test]
async fn scope_scan_only_sends_only_tool_rows_and_no_tools() {
    let mut data = tool_exchange_body();
    data.insert(
        "tools".to_owned(),
        serde_json::from_value(json!([
            {"name": "get", "description": "d", "input_schema": {"type": "object", "properties": {}}}
        ]))
        .expect("request tools"),
    );
    let scan_only = GuardrailSkipFlags {
        scan_only_tool_results: Some(true),
        ..NO_SKIPS
    };
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_input_messages(
        &mut data,
        &guardrail,
        &context_with(scan_only, GlobalSkipFlags::default()),
        None,
    )
    .await
    .unwrap();
    let recording = guardrail.recording();
    assert_eq!(recording.inputs.texts, Some(vec!["res".to_owned()]));
    let rows = recording
        .inputs
        .structured_messages
        .expect("structured rows");
    assert_eq!(roles(&rows), vec!["tool"]);
    assert!(recording.inputs.tools.is_none());
    assert!(recording.inputs.tool_calls.is_none());
}

#[tokio::test]
async fn tools_chat_tools_are_sent_without_native_tools() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": [
            {"name": "get", "description": "d", "input_schema": {"type": "object", "properties": {}}},
            {"web_search_20250305": {}}
        ]
    }));
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    let tools = guardrail.recording().inputs.tools.expect("tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_type, "function");
}

#[tokio::test]
async fn tools_returned_replace_the_request_tools_and_keep_native_ones() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": [
            {"name": "get", "description": "d", "input_schema": {"type": "object", "properties": {}}},
            {"web_search_20250305": {}}
        ]
    }));
    let answer = GuardrailApiInputs {
        tools: Some(vec![returned_tool("new_tool")]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    let written_tools = to_json(data.get("tools").expect("the written tools"));
    let tools = written_tools.as_array().expect("an array");
    assert_eq!(tools.len(), 2);
    assert!(tools[0].get("web_search_20250305").is_some());
    assert_eq!(tools[1]["name"].as_str(), Some("new_tool"));
    assert_eq!(tools[1]["type"].as_str(), Some("custom"));
    assert_eq!(tools[1]["description"].as_str(), Some("n"));
}

#[tokio::test]
async fn tools_scan_only_keeps_request_tools_and_appends_new_names() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "res"}
        ]}],
        "tools": [
            {"name": "get", "description": "d", "input_schema": {"type": "object", "properties": {}}},
            {"name": "other", "description": "o", "input_schema": {"type": "object", "properties": {}}}
        ]
    }));
    let scan_only = GuardrailSkipFlags {
        scan_only_tool_results: Some(true),
        ..NO_SKIPS
    };
    let answer = GuardrailApiInputs {
        tools: Some(vec![returned_tool("get"), returned_tool("new_tool")]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(
        &mut data,
        &guardrail,
        &context_with(scan_only, GlobalSkipFlags::default()),
        None,
    )
    .await
    .unwrap();
    let written_tools = to_json(data.get("tools").expect("the written tools"));
    let tools = written_tools.as_array().expect("an array");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, vec!["get", "other", "new_tool"]);
    assert_eq!(tools[0]["type"].as_str(), Some("custom"));
    assert_eq!(tools[2]["type"].as_str(), Some("custom"));
}

#[tokio::test]
async fn structured_path_rewrites_rows_from_the_full_request_answer() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "system": "trusted",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let answer = GuardrailApiInputs {
        structured_messages: Some(vec![row("system", "trusted"), row("user", "changed")]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    assert_eq!(
        to_json(data.get("system").expect("the system prompt")),
        json!("trusted")
    );
    assert_eq!(
        message_json(&data, 0),
        json!({"role": "user", "content": [{"type": "text", "text": "changed"}]})
    );
}

#[tokio::test]
async fn structured_path_drops_the_tail_of_a_shorter_scoped_answer() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [
            {"role": "user", "content": "one"},
            {"role": "user", "content": "two"}
        ]
    }));
    let answer = GuardrailApiInputs {
        structured_messages: Some(vec![row("user", "edited")]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    let written_messages = to_json(data.get("messages").expect("the messages"));
    let messages = written_messages.as_array().expect("an array");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        json!({"role": "user", "content": [{"type": "text", "text": "edited"}]})
    );
}

#[tokio::test]
async fn structured_path_full_request_answer_replaces_unscoped_rows() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "res"}]},
            {"role": "user", "content": "hi"}
        ]
    }));
    let skip_tool = GuardrailSkipFlags {
        skip_tool_message: Some(true),
        ..NO_SKIPS
    };
    let answer = GuardrailApiInputs {
        structured_messages: Some(vec![row("user", "only")]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer).covering_full_request();
    process_input_messages(
        &mut data,
        &guardrail,
        &context_with(skip_tool, GlobalSkipFlags::default()),
        None,
    )
    .await
    .unwrap();
    let written_messages = to_json(data.get("messages").expect("the messages"));
    let messages = written_messages.as_array().expect("an array");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        json!({"role": "user", "content": [{"type": "text", "text": "only"}]})
    );
}

#[tokio::test]
async fn structured_path_unchanged_answer_falls_through_to_the_text_path() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let answer = GuardrailApiInputs {
        texts: Some(vec!["changed".to_owned()]),
        structured_messages: Some(vec![row("user", "hi")]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    assert_eq!(message_json(&data, 0)["content"].as_str(), Some("changed"));
}

#[tokio::test]
async fn text_path_rewrites_the_system_prompt_and_messages() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "system": "trusted",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let answer = GuardrailApiInputs {
        texts: Some(vec!["s".to_owned(), "u".to_owned()]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    assert_eq!(
        to_json(data.get("system").expect("the system prompt")),
        json!("s")
    );
    assert_eq!(message_json(&data, 0)["content"].as_str(), Some("u"));
}

#[tokio::test]
async fn text_path_rewrites_tool_use_inputs() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": [
            {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {"a": 1}}
        ]}]
    }));
    let rewritten = json!({
        "id": "toolu_1",
        "type": "function",
        "function": {"name": "get", "arguments": "{\"a\": 2}"}
    });
    let answer = GuardrailApiInputs {
        tool_calls: Some(vec![
            serde_json::from_value(rewritten).expect("a tool call"),
        ]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    let message = message_json(&data, 0);
    assert_eq!(message["content"][0]["input"]["a"].as_u64(), Some(2));
}

#[tokio::test]
async fn mismatch_shorter_non_empty_texts_are_unappliable() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": [
            {"type": "text", "text": "a"},
            {"type": "text", "text": "b"}
        ]}]
    }));
    let answer = GuardrailApiInputs {
        texts: Some(vec!["x".to_owned()]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    let error = process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessInputError::Unappliable(_)));
}

#[tokio::test]
async fn text_path_empty_texts_rewrite_nothing() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let before = data.clone();
    let answer = GuardrailApiInputs {
        texts: Some(Vec::new()),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    assert_eq!(data, before);
}

#[tokio::test]
async fn mismatch_non_empty_texts_of_a_different_length_are_unappliable() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let answer = GuardrailApiInputs {
        texts: Some(vec!["a".to_owned(), "b".to_owned()]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    let error = process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessInputError::Unappliable(_)));
}

#[tokio::test]
async fn mismatch_absent_texts_with_returned_tool_calls_are_accepted() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": [
            {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {"a": 1}}
        ]}]
    }));
    let rewritten = json!({
        "id": "toolu_1",
        "type": "function",
        "function": {"name": "get", "arguments": "{\"a\": 2}"}
    });
    let answer = GuardrailApiInputs {
        tool_calls: Some(vec![
            serde_json::from_value(rewritten).expect("a tool call"),
        ]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    assert_eq!(
        message_json(&data, 0)["content"][0]["input"]["a"].as_u64(),
        Some(2)
    );
}

#[tokio::test]
async fn structured_path_keeps_a_midturn_system_row_answer() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [
            {"role": "user", "content": "hi"},
            {"role": "system", "content": "mid"}
        ]
    }));
    let answer = GuardrailApiInputs {
        structured_messages: Some(vec![row("user", "edited"), row("system", "mid2")]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer).covering_full_request();
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    let written = to_json(data.get("messages").expect("the messages"));
    let messages = written.as_array().expect("an array");
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0],
        json!({"role": "user", "content": [{"type": "text", "text": "edited"}]})
    );
    assert_eq!(messages[1], json!({"role": "system", "content": "mid2"}));
}

#[tokio::test]
async fn structured_path_drops_a_system_row_the_request_never_had() {
    let mut data = body(json!({
        "model": "claude-sonnet-4-5",
        "messages": [{"role": "user", "content": "hi"}]
    }));
    let answer = GuardrailApiInputs {
        structured_messages: Some(vec![row("user", "edited"), row("system", "added")]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer).covering_full_request();
    process_input_messages(&mut data, &guardrail, &context(), None)
        .await
        .unwrap();
    let written = to_json(data.get("messages").expect("the messages"));
    assert_eq!(
        written,
        json!([{"role": "user", "content": [{"type": "text", "text": "edited"}]}])
    );
}

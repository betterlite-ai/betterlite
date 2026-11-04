use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::builders::{
    Field, InterpreterInput, SearchInput, SearchResult, build_interpreter_logs,
    build_web_search_call,
};
use betterlite_openai::api::responses_shapes::{
    CodeInterpreterCallType, OutputCallStatus, OutputCodeInterpreterCall,
};
use indexmap::IndexMap;

use super::AnthropicContentReply;
use super::wire::{AnthropicCompletionResponse, JsonFields, get, json_text};

pub(super) fn project(
    response: &AnthropicCompletionResponse,
    reply: &AnthropicContentReply,
) -> JsonFields {
    let mut fields = IndexMap::new();
    fields.insert(
        "citations".to_owned(),
        optional_list(reply.citations.as_ref()),
    );
    fields.insert(
        "thinking_blocks".to_owned(),
        optional_list(reply.thinking_blocks.as_ref()),
    );
    copy_if_present(response, &mut fields, "context_management");
    project_web_search(response, reply, &mut fields);
    project_interpreters(response, reply, &mut fields);
    copy_if_present(response, &mut fields, "container");
    if let Some(blocks) = reply.compaction_blocks.as_ref() {
        fields.insert("compaction_blocks".to_owned(), Json::List(blocks.clone()));
    }
    fields
}

fn optional_list(values: Option<&Vec<Json>>) -> Json {
    values
        .as_ref()
        .map_or(Json::Null, |items| Json::List((*items).clone()))
}

fn copy_if_present(response: &AnthropicCompletionResponse, target: &mut JsonFields, key: &str) {
    if let Some(value) = response
        .fields
        .get(key)
        .filter(|value| !matches!(value, Json::Null))
    {
        target.insert(key.to_owned(), value.clone());
    }
}

fn project_web_search(
    response: &AnthropicCompletionResponse,
    reply: &AnthropicContentReply,
    fields: &mut JsonFields,
) {
    let Some(results) = reply.web_search_results.as_ref() else {
        return;
    };
    fields.insert("web_search_results".to_owned(), Json::List(results.clone()));
    let calls = results
        .iter()
        .filter_map(|result| make_web_search_call(response, result))
        .collect();
    fields.insert("web_search_calls".to_owned(), Json::List(calls));
}

fn make_web_search_call(response: &AnthropicCompletionResponse, result: &Json) -> Option<Json> {
    if get(result, "type").and_then(json_text) != Some("web_search_tool_result") {
        return None;
    }
    let id = get(result, "tool_use_id").and_then(json_text)?;
    let input = latest_search_input(response, id)?;
    let typed_input =
        serde_json::from_str::<Field<SearchInput>>(&serde_json::to_string(input).ok()?)
            .unwrap_or_default();
    let typed_result =
        serde_json::from_str::<Field<SearchResult>>(&serde_json::to_string(result).ok()?)
            .unwrap_or_default();
    let call = build_web_search_call(id, &typed_input, &typed_result, None);
    serde_json::from_str(&serde_json::to_string(&call).ok()?).ok()
}

fn latest_search_input<'response>(
    response: &'response AnthropicCompletionResponse,
    id: &str,
) -> Option<&'response Json> {
    response.content.iter().rev().find_map(|block| {
        if get(block, "type").and_then(json_text) == Some("server_tool_use")
            && get(block, "name").and_then(json_text) == Some("web_search")
            && get(block, "id").and_then(json_text) == Some(id)
        {
            get(block, "input")
        } else {
            None
        }
    })
}

fn project_interpreters(
    response: &AnthropicCompletionResponse,
    reply: &AnthropicContentReply,
    fields: &mut JsonFields,
) {
    if let Some(results) = reply.tool_results.as_ref() {
        add_interpreter_fields(response, results, &reply.tool_calls, fields);
    }
}

fn add_interpreter_fields(
    response: &AnthropicCompletionResponse,
    results: &[Json],
    tool_calls: &[super::wire::AnthropicToolCall],
    fields: &mut JsonFields,
) {
    fields.insert("tool_results".to_owned(), Json::List(results.to_vec()));
    let calls = interpreter_calls(response, results, tool_calls);
    fields.insert("code_interpreter_results".to_owned(), Json::List(calls));
}

fn interpreter_calls(
    response: &AnthropicCompletionResponse,
    results: &[Json],
    tool_calls: &[super::wire::AnthropicToolCall],
) -> Vec<Json> {
    let container_id = response
        .fields
        .get("container")
        .and_then(|value| get(value, "id"))
        .and_then(json_text)
        .map(str::to_owned);
    let code_by_id = interpreter_code_map(tool_calls);
    results
        .iter()
        .filter_map(|result| make_interpreter_call(result, container_id.as_deref(), &code_by_id))
        .collect()
}

fn interpreter_code_map(tool_calls: &[super::wire::AnthropicToolCall]) -> BTreeMap<&str, String> {
    let mut commands = BTreeMap::new();
    for call in tool_calls {
        let Some(id) = call.id.as_deref() else {
            continue;
        };
        let Some(arguments) = call.function.arguments.as_deref() else {
            continue;
        };
        let Ok(Json::Object(args)) = super::decode_json_mode_arguments(arguments) else {
            continue;
        };
        let command = match args.get("command") {
            Some(Json::Text(command)) => command.clone(),
            Some(_) | None => String::new(),
        };
        commands.insert(id, command);
    }
    commands
}

fn make_interpreter_call(
    result: &Json,
    container_id: Option<&str>,
    code_by_id: &BTreeMap<&str, String>,
) -> Option<Json> {
    if get(result, "type").and_then(json_text) != Some("bash_code_execution_tool_result") {
        return None;
    }
    let id = get(result, "tool_use_id")
        .and_then(json_text)
        .unwrap_or_default();
    let content = get(result, "content").cloned().unwrap_or(Json::Null);
    let input =
        serde_json::from_str::<InterpreterInput>(&serde_json::to_string(&content).ok()?).ok()?;
    let outputs = build_interpreter_logs(&input);
    let call = OutputCodeInterpreterCall {
        r#type: CodeInterpreterCallType::CodeInterpreterCall,
        id: id.to_owned(),
        code: Some(code_by_id.get(id).cloned().unwrap_or_default()),
        container_id: container_id.map(str::to_owned),
        status: OutputCallStatus::Completed,
        outputs,
        extra: BTreeMap::new(),
    };
    serde_json::from_str(&serde_json::to_string(&call).ok()?).ok()
}

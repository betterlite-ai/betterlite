use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::response_content::{
    AnthropicCompletionResponse, decode_json_mode_arguments, project_response,
};

fn response(source: &str) -> AnthropicCompletionResponse {
    serde_json::from_str(source).unwrap()
}

#[test]
fn case_0_text_and_citations() {
    let reply = project_response(&response(r#"{"content":[{"type":"text","text":"a"},{"type":"text","text":"b","citations":[{"type":"char_location","supported_text":"old"}]},{"type":"tool_search_tool_result","citations":[{"id":"skip"}]}]}"#), None).unwrap();
    assert_eq!(reply.content, "ab");
    assert_eq!(
        reply.citations,
        Some(vec![
            serde_json::from_str(r#"[{"type":"char_location","supported_text":"b"}]"#).unwrap()
        ])
    );
}

#[test]
fn case_0_tool_use_index() {
    let reply = project_response(&response(r#"{"content":[{"type":"text","text":""},{"type":"tool_use","id":"i","name":"f","input":{"x":1},"caller":null}]}"#), None).unwrap();
    assert_eq!(reply.tool_calls[0].index, 1);
    assert_eq!(reply.tool_calls[0].caller, Some(Json::Null));
}

#[test]
fn case_0_thinking_redacted_content() {
    let reply = project_response(&response(r#"{"content":[{"type":"thinking","thinking":"a"},{"type":"redacted_thinking","data":"x"}]}"#), None).unwrap();
    assert_eq!(reply.reasoning_content.as_deref(), Some("a"));
    assert_eq!(reply.thinking_blocks.unwrap().len(), 2);
}

#[test]
fn case_0_json_tool_only() {
    let reply = project_response(
        &response(
            r#"{"content":[{"type":"tool_use","id":"j","name":"json_tool_call","input":{"values":false}}]}"#,
        ),
        Some(true),
    )
    .unwrap();
    assert!(reply.tool_calls.is_empty());
    assert_eq!(reply.extra_content.as_deref(), Some("false"));
}

#[rstest]
#[case::null_mode(None)]
#[case::false_mode(Some(false))]
fn json_tool_is_kept_when_mode_is_not_true(#[case] mode: Option<bool>) {
    let reply = project_response(
        &response(
            r#"{"content":[{"type":"tool_use","id":"j","name":"json_tool_call","input":{}}]}"#,
        ),
        mode,
    )
    .unwrap();
    assert_eq!(reply.tool_calls.len(), 1);
    assert_eq!(reply.extra_content, None);
}

#[test]
fn case_0_json_tool_with_real_tools() {
    let reply = project_response(&response(r#"{"content":[{"type":"tool_use","id":"j","name":"json_tool_call","input":{"values":0}},{"type":"tool_use","id":"r","name":"run","input":{}}]}"#), Some(true)).unwrap();
    assert_eq!(reply.tool_calls.len(), 1);
    assert_eq!(reply.extra_content.as_deref(), Some("0"));
    assert_eq!(reply.tool_calls[0].function.name.as_deref(), Some("run"));
}

#[test]
fn tool_use_missing_input_returns_error() {
    let error = project_response(
        &response(r#"{"content":[{"type":"tool_use","id":"j","name":"json_tool_call"}]}"#),
        Some(true),
    )
    .unwrap_err();
    assert!(error.to_string().contains("input"));
}

#[test]
fn tool_use_missing_id_returns_error() {
    let error = project_response(
        &response(r#"{"content":[{"type":"tool_use","name":"run","input":{}}]}"#),
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("id"));
}

#[test]
fn tool_use_missing_name_returns_error() {
    let error = project_response(
        &response(r#"{"content":[{"type":"tool_use","id":"r","input":{}}]}"#),
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("name"));
}

#[test]
fn text_missing_string_returns_error() {
    let error = project_response(&response(r#"{"content":[{"type":"text"}]}"#), None).unwrap_err();
    assert!(error.to_string().contains("text"));
}

#[test]
fn case_0_web_search_tool_results() {
    let reply = project_response(&response(r#"{"content":[{"type":"server_tool_use","id":"s","name":"web_search","input":{"query":"first"}},{"type":"server_tool_use","id":"s","name":"web_search","input":{"query":"second"}},{"type":"server_tool_use","id":"s","name":"web_search","input":"invalid"},{"type":"tool_use","id":"s","name":"web_search","input":{"query":"wrong"}},{"type":"web_search_tool_result","tool_use_id":"s","content":[{"type":"web_search_result","url":"https://source.test"}]},{"type":"web_fetch_tool_result","tool_use_id":"s"}]}"#), None).unwrap();
    assert_eq!(reply.web_search_results.unwrap().len(), 2);
    let Json::List(calls) = &reply.provider_specific_fields["web_search_calls"] else {
        panic!("calls list expected");
    };
    assert_eq!(calls.len(), 1);
    let Json::Object(call) = &calls[0] else {
        panic!("web search call object expected");
    };
    let Json::Object(action) = &call["action"] else {
        panic!("web search action object expected");
    };
    assert_eq!(action.get("query"), Some(&Json::Text("second".to_owned())));
    let Json::List(sources) = &action["sources"] else {
        panic!("web search sources list expected");
    };
    assert_eq!(sources.len(), 1);
}

#[test]
fn case_0_code_interpreter_output() {
    let reply = project_response(&response(r#"{"container":{"id":"c"},"content":[{"type":"tool_use","id":"x","name":"bash","input":{"command":"ls"}},{"type":"bash_code_execution_tool_result","tool_use_id":"x","content":{"stdout":"ok","stderr":"bad"}},{"type":"my_custom_tool_result","content":"raw"},{"type":"unknown","content":"not a tool result"},{"type":"tool_search_tool_result"}]}"#), None).unwrap();
    assert_eq!(reply.tool_results.unwrap().len(), 2);
    let Json::List(calls) = &reply.provider_specific_fields["code_interpreter_results"] else {
        panic!("calls list expected");
    };
    let Json::Object(call) = &calls[0] else {
        panic!("call object expected");
    };
    assert_eq!(call.get("code"), Some(&Json::Text("ls".to_owned())));
    assert_eq!(call.get("container_id"), Some(&Json::Text("c".to_owned())));
}

#[test]
fn json_mode_filters_calls_after_code_interpreter_projection() {
    let reply = project_response(&response(r#"{"content":[{"type":"tool_use","id":"t1","name":"json_tool_call","input":{"command":"ls"}},{"type":"bash_code_execution_tool_result","tool_use_id":"t1","content":{"stdout":"ok"}}]}"#), Some(true)).unwrap();
    assert!(reply.tool_calls.is_empty());
    let Json::List(calls) = &reply.provider_specific_fields["code_interpreter_results"] else {
        panic!("calls list expected")
    };
    let Json::Object(call) = &calls[0] else {
        panic!("call object expected")
    };
    assert_eq!(call.get("code"), Some(&Json::Text("ls".to_owned())));
}

#[test]
fn case_0_compaction_provider_field() {
    let reply = project_response(&response(r#"{"context_management":{"edits":[]},"container":{"id":"c"},"content":[{"type":"compaction","opaque":1}]}"#), None).unwrap();
    assert_eq!(reply.compaction_blocks.unwrap().len(), 1);
    assert!(
        reply
            .provider_specific_fields
            .contains_key("context_management")
    );
    assert!(reply.provider_specific_fields.contains_key("container"));
}

#[test]
fn decode_json_mode_arguments_rejects_truncated_input() {
    let error = decode_json_mode_arguments("{\"x\":").unwrap_err();
    assert!(error.to_string().contains("invalid JSON tool arguments"));
}

#[test]
fn replays_the_anthropic_response_content_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_response_content");
    let sources = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let source = fs::read_to_string(path.clone()).unwrap();
            (path, source)
        })
        .collect::<Vec<_>>();
    assert!(sources.iter().any(|(path, source)| {
        path.file_name()
            .is_some_and(|name| name == "valid.json" || name == "boundary.json")
            && decode_json_mode_arguments(source).is_ok()
    }));
    assert!(sources.iter().any(|(path, source)| {
        path.file_name()
            .is_some_and(|name| name == "malformed.json" || name == "truncated.json")
            && decode_json_mode_arguments(source).is_err()
    }));
}

#[rstest]
#[case::json_mode(true)]
fn json_mode_corpus_cases(#[case] enabled: bool) {
    let input = if enabled { r#"{"values":false}"# } else { "{" };
    assert_eq!(decode_json_mode_arguments(input).is_ok(), enabled);
}
use std::fs;
use std::path::Path;

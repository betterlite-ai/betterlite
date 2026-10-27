#![cfg(test)]

use super::ollama;
use crate::builtin::{Content, Message, OllamaPrompt, PromptError, ToolCall, ToolFunction};
use rstest::rstest;
use std::{fs, path::PathBuf};

#[rstest]
#[case::ollama(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some(r####"{"prompt":"### System:\ns\n\n### User:\nu\n\n### Assistant:\na\n\n","images":[]}"####))]
#[case::ollama_merge(r#"[{"role":"user","content":"u"},{"role":"tool","content":"t"},{"role":"function","content":"f"},{"role":"system","content":"s"},{"role":"system","content":"y"},{"role":"assistant","content":"a"},{"role":"assistant","content":"b"}]"#, Some(r####"{"prompt":"### User:\nutf\n\n### System:\nsy\n\n### Assistant:\nab\n\n","images":[]}"####))]
#[case::ollama_empty(r#"[{"role":"user","content":""},{"role":"system","content":[]},{"role":"assistant","content":""}]"#, Some(r#"{"prompt":"","images":[]}"#))]
#[case::ollama_images(r#"[{"role":"user","content":[{"type":"image_url","image_url":"inline"},{"type":"image_url","image_url":{"url":"data:image/png;base64,QQ=="}},{"type":"text","text":"see"}]},{"role":"system","content":[{"type":"text","text":"s"}]},{"role":"assistant","content":[{"type":"text","text":"a"}]}]"#, Some(r####"{"prompt":"### User:\nsee\n\n### System:\ns\n\n### Assistant:\na\n\n","images":["inline","data:image/png;base64,QQ=="]}"####))]
#[case::ollama_role(r#"[{"role":"other","content":"x"}]"#, None)]
#[case::ollama_tools(r#"[{"role":"assistant","content":"a","tool_calls":[{"id":"one","type":"ignored","function":{"name":"\u00e9\ud83d\ude00","arguments":"{\"z\": [1, true, null], \"a\": {\"text\": \"\u00e9\"}}"}},{"id":"two","type":"function","function":{"name":"next","arguments":"{}"}}]}]"#, Some(r####"{"prompt":"### Assistant:\naTool Calls: [\n  {\n    \"id\": \"one\",\n    \"type\": \"function\",\n    \"function\": {\n      \"name\": \"\\u00e9\\ud83d\\ude00\",\n      \"arguments\": {\n        \"z\": [\n          1,\n          true,\n          null\n        ],\n        \"a\": {\n          \"text\": \"\\u00e9\"\n        }\n      }\n    }\n  },\n  {\n    \"id\": \"two\",\n    \"type\": \"function\",\n    \"function\": {\n      \"name\": \"next\",\n      \"arguments\": {}\n    }\n  }\n]\n\n","images":[]}"####))]
#[case::ollama_bad_arguments(r#"[{"role":"assistant","content":"","tool_calls":[{"id":"one","type":"ignored","function":{"name":"f","arguments":"{"}}]}]"#, None)]
#[case::ollama_null(
    r#"[{"role":"assistant","content":null}]"#,
    Some(r#"{"prompt":"","images":[]}"#)
)]
#[case::ollama_trailing_arguments(r#"[{"role":"assistant","content":"a","tool_calls":[{"id":"i","type":"function","function":{"name":"f","arguments":"{} false"}}]}]"#, None)]
#[case::ollama_ignore_parts(r#"[{"role":"user","content":[{"type":"unknown","text":"drop"},{"type":"text"},{"type":"image_url"}]},{"role":"system","content":[{"type":"image_url","image_url":"ignored"}]},{"role":"assistant","content":[{"type":"unknown","text":"kept"}]}]"#, None)]
#[case::numbers(r#"[{"role":"assistant","content":"","tool_calls":[{"id":"i","type":"function","function":{"name":"f","arguments":"{\"small\":1e-5,\"large\":1e16,\"overflow\":1e400,\"int\":999999999999999999999999999,\"zero\":-0}"}}]}]"#, Some(r####"{"prompt":"### Assistant:\nTool Calls: [\n  {\n    \"id\": \"i\",\n    \"type\": \"function\",\n    \"function\": {\n      \"name\": \"f\",\n      \"arguments\": {\n        \"small\": 1e-05,\n        \"large\": 1e+16,\n        \"overflow\": Infinity,\n        \"int\": 999999999999999999999999999,\n        \"zero\": 0\n      }\n    }\n  }\n]\n\n","images":[]}"####))]
#[case::nonstandard_number(r#"[{"role":"assistant","content":"","tool_calls":[{"id":"i","type":"function","function":{"name":"f","arguments":"NaN"}}]}]"#, None)]
#[case::negative_and_delete_character(r#"[{"role":"assistant","content":"","tool_calls":[{"id":"\u007f","type":"function","function":{"name":"f","arguments":"-1e400"}}]}]"#, Some(r####"{"prompt":"### Assistant:\nTool Calls: [\n  {\n    \"id\": \"\\u007f\",\n    \"type\": \"function\",\n    \"function\": {\n      \"name\": \"f\",\n      \"arguments\": -Infinity\n    }\n  }\n]\n\n","images":[]}"####))]
fn builtin_prompts_behavior_tests_behavior(#[case] wire: &str, #[case] expected: Option<&str>) {
    let messages: Vec<Message> = serde_json::from_str(wire).unwrap();
    let actual = ollama("injected-model", &messages);
    match expected {
        Some(text) => assert_eq!(
            actual.unwrap(),
            serde_json::from_str::<OllamaPrompt>(text).unwrap()
        ),
        None => assert!(matches!(
            actual,
            Err(PromptError::Role { .. }
                | PromptError::ToolArguments { .. }
                | PromptError::MissingPart { .. })
        )),
    }
}

#[rstest]
fn replays_the_tool_arguments_corpus(#[files("fuzz/corpus/tool_arguments/*")] path: PathBuf) {
    let arguments = fs::read_to_string(path).unwrap();
    let messages = [Message {
        role: "assistant".to_owned(),
        content: Content::Text(String::new()),
        tool_calls: vec![ToolCall {
            id: "id".to_owned(),
            kind: "function".to_owned(),
            function: ToolFunction {
                name: "f".to_owned(),
                arguments,
            },
        }],
    }];
    drop(ollama("model", &messages));
}

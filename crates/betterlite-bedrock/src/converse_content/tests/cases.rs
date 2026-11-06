#![cfg(test)]

use std::cell::Cell;
use std::slice::from_ref;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;
use uuid::Uuid;

use super::logs::capture;
use crate::converse_content::citations::{
    CitationAnnotation, CitationBlocks, UrlCitation, citations_to_annotations,
};
use crate::converse_content::json_mode::{filter_json_mode_tools, unwrap_bedrock_properties};
use crate::converse_content::recovery::{
    AssistantMessage, MessageFunction, MessageToolCall, recover_tool_call,
};
use crate::converse_content::translation::translate_message_content;
use crate::converse_content::{
    ContentBlock, ConverseContentError, NamedFunction, ToolBlock, ToolCall, ToolCallFunction,
    ToolSpec, get_tool_call_names,
};

#[rstest]
#[case::case_1_reverse_tool_names(case_1_reverse_tool_names)]
#[case::case_2_json_mode_filter(case_2_json_mode_filter)]
#[case::case_3_wrapped_properties(case_3_wrapped_properties)]
#[case::case_4_text_with_tool_use(case_4_text_with_tool_use)]
#[case::case_5_empty_tool_input(case_5_empty_tool_input)]
#[case::case_6_content_function_recovery(case_6_content_function_recovery)]
#[case::case_7_citation_search_locations(case_7_citation_search_locations)]
fn converse_content_cases(#[case] check: fn()) {
    check();
}

fn tool_block(tool_spec: Option<&str>, function: Option<&str>) -> ToolBlock {
    ToolBlock {
        tool_spec: tool_spec.map(|name| ToolSpec {
            name: Some(name.to_owned()),
        }),
        function: function.map(|name| NamedFunction {
            name: Some(name.to_owned()),
        }),
    }
}

fn tool_call(name: Option<&str>, arguments: Option<&str>, index: usize) -> ToolCall {
    ToolCall {
        id: Some(format!("id-{index}")),
        kind: "function".to_owned(),
        function: ToolCallFunction {
            name: name.map(str::to_owned),
            arguments: arguments.map(str::to_owned),
        },
        index,
    }
}

fn function_tools() -> Vec<ToolBlock> {
    vec![
        tool_block(Some("alpha"), None),
        tool_block(None, Some("beta")),
    ]
}

fn case_1_reverse_tool_names() {
    assert!(get_tool_call_names(None).is_empty());
    assert!(get_tool_call_names(Some(&[])).is_empty());
    let tools = vec![
        tool_block(Some("alpha"), None),
        tool_block(None, Some("beta")),
        tool_block(Some("alpha"), Some("gamma")),
        tool_block(Some(""), None),
        ToolBlock {
            tool_spec: Some(ToolSpec { name: None }),
            function: Some(NamedFunction { name: None }),
        },
    ];
    assert_eq!(
        get_tool_call_names(Some(&tools)),
        vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "gamma".to_owned(),
            String::new()
        ]
    );
    assert_eq!(
        get_tool_call_names(Some(&tools))
            .iter()
            .filter(|name| name.is_empty())
            .count(),
        1
    );
}

fn case_2_json_mode_filter() {
    json_mode_filter_unchanged();
    json_mode_filter_synthetic_only();
    json_mode_filter_mixed();
}

fn json_mode_filter_unchanged() {
    let real = tool_call(Some("real"), Some(r#"{"x": 1}"#), 0);
    let disabled = filter_json_mode_tools(false, from_ref(&real), Some("keep"));
    assert_eq!(disabled.tools, Some(vec![real.clone()]));
    assert_eq!(disabled.content.as_deref(), Some("keep"));

    let empty = filter_json_mode_tools(true, &[], Some("keep"));
    assert_eq!(empty.tools, None);
    assert_eq!(empty.content.as_deref(), Some("keep"));

    let unchanged = filter_json_mode_tools(true, from_ref(&real), Some("keep"));
    assert_eq!(unchanged.tools, Some(vec![real]));
    assert_eq!(unchanged.content.as_deref(), Some("keep"));
}

fn json_mode_filter_synthetic_only() {
    let synthetic = vec![tool_call(
        Some("json_tool_call"),
        Some(r#"{"properties": {"a": 1}}"#),
        0,
    )];
    let (logs, _guard) = capture();
    let replaced = filter_json_mode_tools(true, &synthetic, Some("old"));
    assert!(
        logs.text()
            .contains("Processing JSON tool call response for response_format")
    );
    assert_eq!(replaced.tools, None);
    assert_eq!(replaced.content.as_deref(), Some(r#"{"a": 1}"#));

    let missing = vec![tool_call(Some("json_tool_call"), None, 0)];
    let kept = filter_json_mode_tools(true, &missing, Some("old"));
    assert_eq!(kept.tools, None);
    assert_eq!(kept.content.as_deref(), Some("old"));

    let no_content = filter_json_mode_tools(
        true,
        &[tool_call(
            Some("json_tool_call"),
            Some(r#"{"properties": 1}"#),
            0,
        )],
        None,
    );
    assert_eq!(no_content.tools, None);
    assert_eq!(no_content.content.as_deref(), Some("1"));
}

fn json_mode_filter_mixed() {
    let real = tool_call(Some("real"), Some(r#"{"x": 1}"#), 0);
    let mixed = vec![
        real.clone(),
        tool_call(Some("json_tool_call"), Some(r#"{"properties": [3]}"#), 1),
        tool_call(Some("json_tool_call"), Some(r#"{"b": 2}"#), 2),
        tool_call(Some("other"), Some("{}"), 3),
    ];
    let filtered = filter_json_mode_tools(true, &mixed, Some("keep "));
    assert_eq!(filtered.content.as_deref(), Some("keep [3]"));
    assert_eq!(filtered.tools, Some(vec![real.clone(), mixed[3].clone()]));

    let mixed_missing = filter_json_mode_tools(
        true,
        &[real.clone(), tool_call(Some("json_tool_call"), None, 1)],
        Some("keep"),
    );
    assert_eq!(mixed_missing.content.as_deref(), Some("keep"));
    assert_eq!(mixed_missing.tools, Some(vec![real]));
}

fn case_3_wrapped_properties() {
    assert_eq!(
        unwrap_bedrock_properties(r#"{"properties": {"b": 1, "a": 2}}"#),
        r#"{"b": 1, "a": 2}"#
    );
    assert_eq!(unwrap_bedrock_properties(r#"{"properties": null}"#), "null");
    assert_eq!(
        unwrap_bedrock_properties(r#"  {"properties": [1, 2]}  "#),
        "[1, 2]"
    );
    assert_eq!(
        unwrap_bedrock_properties(r#"{"properties": "h\u00e9llo"}"#),
        r#""h\u00e9llo""#
    );
    assert_eq!(
        unwrap_bedrock_properties(r#"{"properties": {"properties": {"a": 1}}}"#),
        r#"{"properties": {"a": 1}}"#
    );
    assert_eq!(
        unwrap_bedrock_properties(r#"  {"properties": 1, "other": 2}  "#),
        r#"  {"properties": 1, "other": 2}  "#
    );
    assert_eq!(unwrap_bedrock_properties("not json"), "not json".to_owned());
    assert_eq!(unwrap_bedrock_properties("[1]"), "[1]");
    assert_eq!(unwrap_bedrock_properties("{}"), "{}");
    assert_eq!(unwrap_bedrock_properties("42"), "42");
    assert_eq!(unwrap_bedrock_properties("null"), "null");
}

const TRANSLATION_BLOCKS: &str = r#"[
    {"text": "<thinking>plan</thinking>Answer"},
    {
        "text": " continues",
        "toolUse": {"toolUseId": "tu-1", "name": "x-tool", "input": {"b": 2, "a": [1, 3]}},
        "reasoningContent": {"reasoningText": {"text": "explicit", "signature": "sig"}},
        "citationsContent": {"content": [{"text": "cited"}], "citations": []}
    },
    {"reasoningContent": {}},
    {"citationsContent": {}},
    {"cachePoint": {"type": "default"}},
    {}
]"#;

fn case_4_text_with_tool_use() {
    text_with_tool_use_projection();
    text_with_tool_use_without_blocks();
}

fn text_with_tool_use_projection() {
    let blocks: Vec<ContentBlock> = serde_json::from_str(TRANSLATION_BLOCKS).unwrap();
    let mut mappings = IndexMap::new();
    mappings.insert("x-tool".to_owned(), "original_tool".to_owned());
    let translation = translate_message_content(&blocks, &mappings).unwrap();
    assert_eq!(translation.content, "Answer continues");
    assert_eq!(translation.tools.len(), 1);
    let call = &translation.tools[0];
    assert_eq!(call.index, 1);
    assert_eq!(call.id.as_deref(), Some("tu-1"));
    assert_eq!(call.kind, "function");
    assert_eq!(call.function.name.as_deref(), Some("original_tool"));
    assert_eq!(
        call.function.arguments.as_deref(),
        Some(r#"{"b": 2, "a": [1, 3]}"#)
    );
    let reasoning: Vec<Json> = serde_json::from_str(
        r#"[{"reasoningText": {"text": "plan"}}, {"reasoningText": {"text": "explicit", "signature": "sig"}}, {}]"#,
    )
    .unwrap();
    assert_eq!(translation.reasoning_content_blocks, Some(reasoning));
    let citations: Vec<Json> =
        serde_json::from_str(r#"[{"content": [{"text": "cited"}], "citations": []}, {}]"#).unwrap();
    assert_eq!(translation.citations_content_blocks, Some(citations));
}

fn text_with_tool_use_without_blocks() {
    let mappings = IndexMap::new();
    let empty = translate_message_content(&[], &mappings).unwrap();
    assert_eq!(empty.content, "");
    assert!(empty.tools.is_empty());
    assert_eq!(empty.reasoning_content_blocks, None);
    assert_eq!(empty.citations_content_blocks, None);

    let thinking: Vec<ContentBlock> =
        serde_json::from_str(r#"[{"text": "<think>r</think>"}]"#).unwrap();
    let reasoning_only = translate_message_content(&thinking, &mappings).unwrap();
    assert_eq!(reasoning_only.content, "");
    let extracted: Vec<Json> =
        serde_json::from_str(r#"[{"reasoningText": {"text": "r"}}]"#).unwrap();
    assert_eq!(reasoning_only.reasoning_content_blocks, Some(extracted));
}

const EMPTY_INPUT_BLOCKS: &str = r#"[
    {"text": "prefix"},
    {"toolUse": {"toolUseId": "tu-empty", "name": "plain", "input": {}}},
    {"toolUse": {"toolUseId": "tu-null", "name": "plain", "input": null}},
    {"toolUse": {"toolUseId": "tu-nested", "name": "plain", "input": {"b": [1, {"d": 4}], "a": null}}}
]"#;

fn case_5_empty_tool_input() {
    empty_tool_input_arguments();
    empty_tool_input_errors();
}

fn empty_tool_input_arguments() {
    let blocks: Vec<ContentBlock> = serde_json::from_str(EMPTY_INPUT_BLOCKS).unwrap();
    let mappings = IndexMap::new();
    let translation = translate_message_content(&blocks, &mappings).unwrap();
    assert_eq!(translation.content, "prefix");
    assert_eq!(translation.tools.len(), 3);
    let arguments: Vec<Option<&str>> = translation
        .tools
        .iter()
        .map(|call| call.function.arguments.as_deref())
        .collect();
    assert_eq!(
        arguments,
        vec![
            Some("{}"),
            Some("null"),
            Some(r#"{"b": [1, {"d": 4}], "a": null}"#)
        ]
    );
    let indices: Vec<usize> = translation.tools.iter().map(|call| call.index).collect();
    assert_eq!(indices, vec![1, 2, 3]);
    let ids: Vec<Option<&str>> = translation
        .tools
        .iter()
        .map(|call| call.id.as_deref())
        .collect();
    assert_eq!(
        ids,
        vec![Some("tu-empty"), Some("tu-null"), Some("tu-nested")]
    );
}

fn empty_tool_input_errors() {
    let mappings = IndexMap::new();
    let missing_input: Vec<ContentBlock> =
        serde_json::from_str(r#"[{"toolUse": {"toolUseId": "t", "name": "n"}}]"#).unwrap();
    assert!(matches!(
        translate_message_content(&missing_input, &mappings),
        Err(ConverseContentError::ToolUseField { field: "input" })
    ));
    let missing_id: Vec<ContentBlock> =
        serde_json::from_str(r#"[{"toolUse": {"name": "n", "input": {}}}]"#).unwrap();
    assert!(matches!(
        translate_message_content(&missing_id, &mappings),
        Err(ConverseContentError::ToolUseField { field: "toolUseId" })
    ));
    let missing_name: Vec<ContentBlock> =
        serde_json::from_str(r#"[{"toolUse": {"toolUseId": "t", "input": {}}}]"#).unwrap();
    assert!(matches!(
        translate_message_content(&missing_name, &mappings),
        Err(ConverseContentError::ToolUseField { field: "name" })
    ));
    let malformed: Vec<ContentBlock> =
        serde_json::from_str(r#"[{"toolUse": "not an object"}]"#).unwrap();
    assert!(matches!(
        translate_message_content(&malformed, &mappings),
        Err(ConverseContentError::ToolUseObject)
    ));
}

fn message(content: Option<&str>) -> AssistantMessage {
    AssistantMessage {
        content: content.map(str::to_owned),
        tool_calls: None,
    }
}

fn recovered_call(arguments: &str) -> MessageToolCall {
    MessageToolCall {
        id: Uuid::nil().to_string(),
        kind: "function".to_owned(),
        function: MessageFunction {
            name: Some("alpha".to_owned()),
            arguments: arguments.to_owned(),
        },
    }
}

fn case_6_content_function_recovery() {
    recovery_with_arguments();
    recovery_failures();
    recovery_without_match();
}

fn recovery_with_arguments() {
    let tools = function_tools();
    let identifier = Uuid::nil;
    let mut recovered = message(Some(
        r#"{"type": "function", "name": "alpha", "arguments": {"b": 1, "a": 2}, "extra": true}"#,
    ));
    let recovered_finish =
        recover_tool_call(&mut recovered, Some(&tools), Some("stop"), &identifier);
    assert_eq!(recovered_finish.as_deref(), Some("tool_calls"));
    assert_eq!(recovered.content, None);
    assert_eq!(
        recovered.tool_calls,
        Some(vec![recovered_call(r#"{"b": 1, "a": 2}"#)])
    );

    let mut absent = message(Some(r#"{"type": "function", "name": "alpha"}"#));
    let absent_finish = recover_tool_call(&mut absent, Some(&tools), Some("stop"), &identifier);
    assert_eq!(absent_finish.as_deref(), Some("tool_calls"));
    assert_eq!(absent.tool_calls, Some(vec![recovered_call("")]));

    let mut parameters = message(Some(
        r#"{"type": "function", "name": "alpha", "arguments": null, "parameters": {"p": 1}}"#,
    ));
    let parameters_finish =
        recover_tool_call(&mut parameters, Some(&tools), Some("stop"), &identifier);
    assert_eq!(parameters_finish.as_deref(), Some("tool_calls"));
    assert_eq!(
        parameters.tool_calls,
        Some(vec![recovered_call(r#"{"p": 1}"#)])
    );
}

fn recovery_failures() {
    let tools = function_tools();
    let failures = [
        r#"{"type": "function", "name": "alpha", "arguments": 5}"#,
        r#"{"type": "function", "name": "gamma", "arguments": "{}"}"#,
        r#"{"type": "other", "name": "alpha", "arguments": "{}"}"#,
        r#"["type", "function"]"#,
        r#""just a string""#,
        "not json",
        "null",
    ];
    for content in failures {
        let called = Cell::new(false);
        let tracking = || {
            called.set(true);
            Uuid::nil()
        };
        let mut unchanged = message(Some(content));
        let failure_finish =
            recover_tool_call(&mut unchanged, Some(&tools), Some("length"), &tracking);
        assert_eq!(failure_finish.as_deref(), Some("length"), "{content}");
        assert_eq!(unchanged.content.as_deref(), Some(content), "{content}");
        assert_eq!(unchanged.tool_calls, None, "{content}");
        assert!(!called.get(), "{content}");
    }
}

fn recovery_without_match() {
    let tools = function_tools();
    let identifier = Uuid::nil;
    let existing = recovered_call("{}");
    let mut kept = AssistantMessage {
        content: Some("not json".to_owned()),
        tool_calls: Some(vec![existing.clone()]),
    };
    let kept_finish = recover_tool_call(&mut kept, Some(&tools), Some("stop"), &identifier);
    assert_eq!(kept_finish.as_deref(), Some("stop"));
    assert_eq!(kept.tool_calls, Some(vec![existing]));

    let mut without_tools = message(Some(r#"{"type": "function", "name": "alpha"}"#));
    let without_tools_finish = recover_tool_call(&mut without_tools, None, None, &identifier);
    assert_eq!(without_tools_finish, None);
    assert_eq!(
        without_tools.content.as_deref(),
        Some(r#"{"type": "function", "name": "alpha"}"#)
    );

    let mut empty_tools = message(Some(r#"{"type": "function", "name": "alpha"}"#));
    let empty_tools_finish = recover_tool_call(&mut empty_tools, Some(&[]), None, &identifier);
    assert_eq!(empty_tools_finish, None);
    assert_eq!(empty_tools.tool_calls, None);

    let mut no_content = AssistantMessage::default();
    let no_content_finish = recover_tool_call(&mut no_content, Some(&tools), None, &identifier);
    assert_eq!(no_content_finish, None);
}

fn citation(start: u8, end: u8, title: &str, url: &str) -> CitationAnnotation {
    CitationAnnotation {
        kind: "url_citation".to_owned(),
        url_citation: UrlCitation {
            start_index: Number::from(start),
            end_index: Number::from(end),
            title: title.to_owned(),
            url: url.to_owned(),
        },
    }
}

const CITATION_BLOCKS: &str = r#"[
    {
        "content": [{"text": "abc"}, "bad", {"text": 7}, {"text": "de"}],
        "citations": [
            {"location": {"searchResultLocation": {"start": 1, "end": true}}, "source": "https://a.test", "title": "A"},
            {"location": {"searchResultLocation": {"start": 10, "end": 12}}, "source": null, "title": ""},
            {"location": {"documentChar": {"start": 0, "end": 1}}},
            {"location": {"searchResultLocation": {"start": 1.5, "end": 2}}}
        ]
    },
    {
        "content": [],
        "citations": [
            {"location": {"searchResultLocation": {"start": 0, "end": 1}}, "source": "https://b.test", "title": "B"}
        ]
    }
]"#;

fn case_7_citation_search_locations() {
    citation_offsets_and_kinds();
    citation_character_offsets();
}

fn citation_offsets_and_kinds() {
    let empty = CitationBlocks {
        text: None,
        annotations: None,
    };
    assert_eq!(citations_to_annotations(None), empty);
    assert_eq!(citations_to_annotations(Some(&[])), empty);
    let blocks: Vec<Json> = serde_json::from_str(CITATION_BLOCKS).unwrap();
    assert_eq!(
        citations_to_annotations(Some(&blocks)),
        CitationBlocks {
            text: Some("abcde".to_owned()),
            annotations: Some(vec![
                citation(1, 1, "A", "https://a.test"),
                citation(10, 12, "", ""),
                citation(5, 6, "B", "https://b.test"),
            ]),
        }
    );
}

fn citation_character_offsets() {
    let unicode: Vec<Json> = serde_json::from_str(
        r#"[{"content": [{"text": "\u00e9"}]}, {"content": [{"text": "x"}], "citations": [{"location": {"searchResultLocation": {"start": 0, "end": 1}}}]}]"#,
    )
    .unwrap();
    assert_eq!(
        citations_to_annotations(Some(&unicode)),
        CitationBlocks {
            text: Some("\u{e9}x".to_owned()),
            annotations: Some(vec![citation(1, 2, "", "")]),
        }
    );
}

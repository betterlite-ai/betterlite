use rstest::rstest;

use crate::sampling_convert::content::{
    McpContentBlock, SamplingContent, SamplingRole, ToolResultBlock,
};
use crate::sampling_convert::json_object::JsonObject;
use crate::sampling_convert::messages::{SamplingMessage, convert_messages};
use crate::sampling_convert::openai_wire::{
    ImageUrl, InputAudio, OpenAiContent, OpenAiContentPart, OpenAiMessage, OpenAiRole,
};

use super::fixtures::{
    expected_call, many, message, object, single, text, text_block, text_parts_message,
    tool_result, tool_use,
};

#[test]
fn text_content_converts_to_a_parts_list() {
    let messages = vec![message(SamplingRole::User, single(text("hi")))];
    assert_eq!(
        convert_messages(&messages, None),
        vec![text_parts_message("hi")]
    );
}

#[test]
fn content_parts_keep_their_order_and_encoding() {
    let content = many(vec![
        text("look"),
        McpContentBlock::Image {
            data: String::from("AAA"),
            mime_type: String::from("image/png"),
        },
        McpContentBlock::Audio {
            data: String::from("BBB"),
            mime_type: String::from("audio/flac"),
        },
    ]);
    let messages = vec![message(SamplingRole::User, content)];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::User,
            content: Some(OpenAiContent::Parts(vec![
                OpenAiContentPart::Text {
                    text: String::from("look"),
                },
                OpenAiContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: String::from("data:image/png;base64,AAA"),
                    },
                },
                OpenAiContentPart::InputAudio {
                    input_audio: InputAudio {
                        data: String::from("BBB"),
                        format: String::from("flac"),
                    },
                },
            ])),
            tool_calls: None,
            tool_call_id: None,
        }]
    );
}

#[rstest]
#[case::wav("audio/wav", "wav")]
#[case::mp3("audio/mp3", "mp3")]
#[case::mpeg("audio/mpeg", "mp3")]
#[case::flac("audio/flac", "flac")]
#[case::ogg("audio/ogg", "ogg")]
#[case::unknown("audio/webm", "wav")]
fn audio_mime_types_map_to_openai_formats(#[case] mime_type: &str, #[case] expected: &str) {
    let content = single(McpContentBlock::Audio {
        data: String::from("BBB"),
        mime_type: mime_type.to_owned(),
    });
    let messages = vec![message(SamplingRole::User, content)];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::User,
            content: Some(OpenAiContent::Parts(vec![OpenAiContentPart::InputAudio {
                input_audio: InputAudio {
                    data: String::from("BBB"),
                    format: expected.to_owned(),
                },
            },])),
            tool_calls: None,
            tool_call_id: None,
        }]
    );
}

#[test]
fn assistant_tool_use_converts_to_tool_calls_without_content() {
    let messages = vec![message(
        SamplingRole::Assistant,
        single(tool_use("call-1", "lookup", r#"{"q": 1}"#)),
    )];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::Assistant,
            content: None,
            tool_calls: Some(vec![expected_call("call-1", "lookup", r#"{"q": 1}"#)]),
            tool_call_id: None,
        }]
    );
}

#[test]
fn assistant_text_alongside_tool_use_joins_text_with_newlines() {
    let content = many(vec![
        text("one"),
        text("two"),
        tool_use("call-1", "lookup", "{}"),
    ]);
    let messages = vec![message(SamplingRole::Assistant, content)];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::Assistant,
            content: Some(OpenAiContent::Text(String::from("one\ntwo"))),
            tool_calls: Some(vec![expected_call("call-1", "lookup", "{}")]),
            tool_call_id: None,
        }]
    );
}

#[test]
fn assistant_with_two_tool_uses_keeps_their_order() {
    let content = many(vec![
        tool_use("call-1", "first", "{}"),
        tool_use("call-2", "second", r#"{"nested": {"x": [1, 2]}}"#),
    ]);
    let messages = vec![message(SamplingRole::Assistant, content)];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::Assistant,
            content: None,
            tool_calls: Some(vec![
                expected_call("call-1", "first", "{}"),
                expected_call("call-2", "second", r#"{"nested": {"x": [1, 2]}}"#),
            ]),
            tool_call_id: None,
        }]
    );
}

#[test]
fn user_tool_results_become_tool_messages_and_drop_other_content() {
    let content = many(vec![
        text("ignored"),
        tool_result("call-1", vec![text_block("one"), text_block("two")]),
        tool_result("call-2", vec![text_block("three")]),
    ]);
    let messages = vec![message(SamplingRole::User, content)];
    assert_eq!(
        convert_messages(&messages, None),
        vec![
            OpenAiMessage {
                role: OpenAiRole::Tool,
                content: Some(OpenAiContent::Text(String::from("one\ntwo"))),
                tool_calls: None,
                tool_call_id: Some(String::from("call-1")),
            },
            OpenAiMessage {
                role: OpenAiRole::Tool,
                content: Some(OpenAiContent::Text(String::from("three"))),
                tool_calls: None,
                tool_call_id: Some(String::from("call-2")),
            },
        ]
    );
}

#[test]
fn a_single_tool_result_converts_to_one_tool_message() {
    let messages = vec![message(
        SamplingRole::User,
        single(tool_result("call-1", vec![text_block("only")])),
    )];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::Tool,
            content: Some(OpenAiContent::Text(String::from("only"))),
            tool_calls: None,
            tool_call_id: Some(String::from("call-1")),
        }]
    );
}

#[test]
fn a_tool_result_without_text_has_empty_content() {
    let messages = vec![message(
        SamplingRole::Assistant,
        single(tool_result("call-1", Vec::new())),
    )];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::Tool,
            content: Some(OpenAiContent::Text(String::new())),
            tool_calls: None,
            tool_call_id: Some(String::from("call-1")),
        }]
    );
}

#[test]
fn non_text_tool_result_blocks_are_ignored() {
    let content = vec![text_block("one"), ToolResultBlock::Other, text_block("two")];
    let messages = vec![message(
        SamplingRole::User,
        single(tool_result("call-1", content)),
    )];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::Tool,
            content: Some(OpenAiContent::Text(String::from("one\ntwo"))),
            tool_calls: None,
            tool_call_id: Some(String::from("call-1")),
        }]
    );
}

#[test]
fn a_stray_tool_use_on_a_user_message_becomes_an_assistant_tool_call() {
    let messages = vec![message(
        SamplingRole::User,
        single(tool_use("call-1", "lookup", r#"{"q": 1}"#)),
    )];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::Assistant,
            content: None,
            tool_calls: Some(vec![expected_call("call-1", "lookup", r#"{"q": 1}"#)]),
            tool_call_id: None,
        }]
    );
}

#[test]
fn mixed_user_content_with_a_tool_use_keeps_the_text_parts() {
    let content = many(vec![text("why"), tool_use("call-1", "lookup", "{}")]);
    let messages = vec![message(SamplingRole::User, content)];
    assert_eq!(
        convert_messages(&messages, None),
        vec![OpenAiMessage {
            role: OpenAiRole::Assistant,
            content: Some(OpenAiContent::Parts(vec![OpenAiContentPart::Text {
                text: String::from("why"),
            }])),
            tool_calls: Some(vec![expected_call("call-1", "lookup", "{}")]),
            tool_call_id: None,
        }]
    );
}

#[test]
fn an_empty_content_list_adds_no_message() {
    let messages = vec![message(SamplingRole::User, many(Vec::new()))];
    assert_eq!(convert_messages(&messages, None), Vec::new());
}

#[test]
fn an_empty_text_content_still_converts() {
    let messages = vec![message(SamplingRole::User, single(text("")))];
    assert_eq!(
        convert_messages(&messages, None),
        vec![text_parts_message("")]
    );
}

#[test]
fn a_system_prompt_comes_first() {
    let messages = vec![message(SamplingRole::User, single(text("hi")))];
    let converted = convert_messages(&messages, Some("be nice"));
    assert_eq!(
        converted.first(),
        Some(&OpenAiMessage {
            role: OpenAiRole::System,
            content: Some(OpenAiContent::Text(String::from("be nice"))),
            tool_calls: None,
            tool_call_id: None,
        })
    );
    assert_eq!(converted.len(), 2);
}

#[test]
fn an_empty_system_prompt_is_omitted() {
    let messages = vec![message(SamplingRole::User, single(text("hi")))];
    assert_eq!(
        convert_messages(&messages, Some("")),
        vec![text_parts_message("hi")]
    );
}

#[rstest]
#[case::camel(r#"{"role":"user","content":{"type":"image","data":"AAA","mimeType":"image/png"}}"#)]
#[case::snake(r#"{"role":"user","content":{"type":"image","data":"AAA","mime_type":"image/png"}}"#)]
fn sampling_messages_decode_from_both_mcp_spellings(#[case] document: &str) {
    let message: SamplingMessage = serde_json::from_str(document).expect("message");
    assert_eq!(
        message.content,
        single(McpContentBlock::Image {
            data: String::from("AAA"),
            mime_type: String::from("image/png"),
        })
    );
}

#[test]
fn a_tool_result_message_decodes_from_the_mcp_wire() {
    let document = r#"{"role":"user","content":{"type":"tool_result","toolUseId":"call-1","content":[{"type":"text","text":"one"}]}}"#;
    let message: SamplingMessage = serde_json::from_str(document).expect("message");
    assert_eq!(
        message.content,
        single(McpContentBlock::ToolResult {
            tool_use_id: String::from("call-1"),
            content: vec![text_block("one")],
        })
    );
}

#[test]
fn a_tool_use_message_decodes_from_the_mcp_wire() {
    let document = r#"{"role":"assistant","content":{"type":"tool_use","id":"call-1","name":"lookup","input":{"q": 1}}}"#;
    let message: SamplingMessage = serde_json::from_str(document).expect("message");
    let SamplingContent::Single(McpContentBlock::ToolUse { id, name, input }) = &message.content
    else {
        panic!("expected a tool use block");
    };
    assert_eq!(id, "call-1");
    assert_eq!(name, "lookup");
    assert_eq!(input.as_ref(), r#"{"q": 1}"#);
}

#[test]
fn a_tool_use_input_that_is_not_an_object_is_refused() {
    let document = r#"{"role":"assistant","content":{"type":"tool_use","id":"call-1","name":"lookup","input":[1]}}"#;
    serde_json::from_str::<SamplingMessage>(document).expect_err("refused input");
}

#[test]
fn a_json_object_refuses_a_non_object_document() {
    let error = serde_json::from_str::<JsonObject>("[1]").expect_err("not an object");
    assert!(error.to_string().contains("valid dictionary"));
}

#[test]
fn json_objects_compare_their_text() {
    let left = object(r#"{"q": 1}"#);
    assert_eq!(left, object(r#"{"q": 1}"#));
    assert_ne!(left, object(r#"{"q": 2}"#));
}

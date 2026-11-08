#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::arguments::json::{Json, Object};
use crate::azure::convert::to_azure;
use crate::azure::error::AzureError;
use crate::azure::stringify::stringify_tool_content;
use crate::azure::text::azure_text_pt;
use crate::cohere::history::cohere_messages_pt_v2;
use crate::cohere::prompt::cohere_message_pt;
use crate::media::limits::{MAX_MESSAGE_PARTS, MAX_MESSAGES};
use crate::message_content::{Content, ContentPart, Message};

struct Case {
    messages: &'static str,
    expected: &'static str,
}

struct TextCase {
    messages: &'static str,
    expected: &'static str,
}

struct FailureCase {
    messages: &'static str,
    operation: Operation,
    failure: Failure,
}

#[derive(Clone, Copy)]
enum Failure {
    Index(IndexKind, usize),
    Position(PositionKind, usize, usize),
    ToolContent(usize),
}

#[derive(Clone, Copy)]
enum IndexKind {
    FunctionCall,
    Arguments,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PositionKind {
    MissingImageUrl,
    ImageUrl,
    File,
    Text,
}

#[derive(Clone, Copy)]
enum Operation {
    Convert,
    Text,
    Stringify,
}

enum Scenario {
    Convert(Case),
    Text(TextCase),
    Stringify(Case),
    Failure(FailureCase),
    MessageLimit(Operation),
    MessageAtLimit(Operation),
    PartLimit(Operation),
    PartsAtLimit(Operation),
    Preserved(&'static str),
}

fn parse_messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).unwrap()
}

fn one_message() -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text("a".to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn text_part() -> ContentPart {
    let mut fields = Object::new();
    fields.insert("text".to_owned(), Json::Text("a".to_owned()));
    ContentPart {
        kind: "text".to_owned(),
        fields,
    }
}

fn parts_message(count: usize) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Parts(vec![text_part(); count])),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn run_operation(operation: Operation, messages: &[Message]) -> Result<(), AzureError> {
    match operation {
        Operation::Convert => to_azure(messages).map(drop),
        Operation::Text => azure_text_pt(messages).map(drop),
        Operation::Stringify => stringify_tool_content(messages).map(drop),
    }
}

fn assert_messages(case: &Case, convert: fn(&[Message]) -> Result<Vec<Message>, AzureError>) {
    let messages = parse_messages(case.messages);
    let converted = convert(&messages).unwrap();
    assert_eq!(
        serde_json::to_value(&converted).unwrap(),
        serde_json::to_value(parse_messages(case.expected)).unwrap()
    );
}

fn assert_text(case: &TextCase) {
    let messages = parse_messages(case.messages);
    assert_eq!(azure_text_pt(&messages).unwrap(), case.expected);
}

fn assert_failure(case: &FailureCase) {
    let messages = parse_messages(case.messages);
    let error = run_operation(case.operation, &messages).unwrap_err();
    assert!(matches_failure(&error, case.failure), "unexpected {error}");
}

fn matches_failure(error: &AzureError, failure: Failure) -> bool {
    match failure {
        Failure::Index(kind, expected) => matches_index(error, kind, expected),
        Failure::Position(kind, index, part) => matches_position(error, kind, index, part),
        Failure::ToolContent(expected) => {
            matches!(error, AzureError::MissingToolContent { index } if *index == expected)
        }
    }
}

fn matches_index(error: &AzureError, kind: IndexKind, expected: usize) -> bool {
    let index = match error {
        AzureError::FunctionCall { index } | AzureError::Arguments { index } => index,
        AzureError::TooManyMessages { .. }
        | AzureError::TooManyParts { .. }
        | AzureError::MissingImageUrl { .. }
        | AzureError::ImageUrl { .. }
        | AzureError::File { .. }
        | AzureError::Text { .. }
        | AzureError::MissingToolContent { .. }
        | AzureError::Serialize { .. } => return false,
    };
    let kind_matches = match kind {
        IndexKind::FunctionCall => matches!(error, AzureError::FunctionCall { .. }),
        IndexKind::Arguments => matches!(error, AzureError::Arguments { .. }),
    };
    kind_matches && *index == expected
}

fn matches_position(error: &AzureError, kind: PositionKind, index: usize, part: usize) -> bool {
    match error {
        AzureError::MissingImageUrl {
            index: actual_index,
            part: actual_part,
        } => {
            kind == PositionKind::MissingImageUrl && index == *actual_index && part == *actual_part
        }
        AzureError::ImageUrl {
            index: actual_index,
            part: actual_part,
        } => kind == PositionKind::ImageUrl && index == *actual_index && part == *actual_part,
        AzureError::File {
            index: actual_index,
            part: actual_part,
        } => kind == PositionKind::File && index == *actual_index && part == *actual_part,
        AzureError::Text {
            index: actual_index,
            part: actual_part,
        } => kind == PositionKind::Text && index == *actual_index && part == *actual_part,
        AzureError::TooManyMessages { .. }
        | AzureError::TooManyParts { .. }
        | AzureError::FunctionCall { .. }
        | AzureError::Arguments { .. }
        | AzureError::MissingToolContent { .. }
        | AzureError::Serialize { .. } => false,
    }
}

fn assert_preserved(source: &'static str) {
    let messages = parse_messages(source);
    let untouched = messages.clone();
    let first = to_azure(&messages).unwrap();
    let second = to_azure(&messages).unwrap();
    assert_eq!(messages, untouched);
    assert_eq!(first, second);
}

#[rstest]
#[case::assistant_function_call_gains_empty_arguments(Scenario::Convert(Case {
    messages: r#"[{"role":"assistant","content":"hi","function_call":{"name":"get_time"}}]"#,
    expected: r#"[{"role":"assistant","content":"hi","function_call":{"name":"get_time","arguments":""}}]"#,
}))]
#[case::assistant_null_arguments_gain_empty_string(Scenario::Convert(Case {
    messages: r#"[{"role":"assistant","content":"hi","function_call":{"name":"get_time","arguments":null}}]"#,
    expected: r#"[{"role":"assistant","content":"hi","function_call":{"name":"get_time","arguments":""}}]"#,
}))]
#[case::assistant_existing_arguments_are_kept(Scenario::Convert(Case {
    messages: r#"[{"role":"assistant","content":"hi","function_call":{"name":"f","arguments":"{\"a\":1}"}}]"#,
    expected: r#"[{"role":"assistant","content":"hi","function_call":{"name":"f","arguments":"{\"a\":1}"}}]"#,
}))]
#[case::null_function_call_is_left_alone(Scenario::Convert(Case {
    messages: r#"[{"role":"assistant","content":"hi","function_call":null}]"#,
    expected: r#"[{"role":"assistant","content":"hi","function_call":null}]"#,
}))]
#[case::assistant_tool_calls_and_other_fields_are_kept(Scenario::Convert(Case {
    messages: r#"[{"role":"assistant","content":"hi","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}],"cache_control":{"type":"ephemeral"}}]"#,
    expected: r#"[{"role":"assistant","content":"hi","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}],"cache_control":{"type":"ephemeral"}}]"#,
}))]
#[case::user_image_url_string_wraps_in_a_url_object(Scenario::Convert(Case {
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":"https://example.test/a.png","detail":"high"}]}]"#,
    expected: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"},"detail":"high"}]}]"#,
}))]
#[case::user_image_url_object_loses_format(Scenario::Convert(Case {
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png","detail":"low","format":"png"}}]}]"#,
    expected: r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png","detail":"low"}}]}]"#,
}))]
#[case::user_file_loses_format(Scenario::Convert(Case {
    messages: r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"data:application/pdf;base64,QQ==","format":"pdf","filename":"a.pdf"}}]}]"#,
    expected: r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"data:application/pdf;base64,QQ==","filename":"a.pdf"}}]}]"#,
}))]
#[case::user_file_without_file_becomes_an_empty_object(Scenario::Convert(Case {
    messages: r#"[{"role":"user","content":[{"type":"file","filename":"a.pdf"}]}]"#,
    expected: r#"[{"role":"user","content":[{"type":"file","filename":"a.pdf","file":{}}]}]"#,
}))]
#[case::other_parts_and_roles_are_untouched(Scenario::Convert(Case {
    messages: r#"[{"role":"system","content":[{"type":"image_url","image_url":"https://example.test/a.png"}]},{"role":"user","content":[{"type":"text","text":"t"},{"type":"input_audio","input_audio":{"data":"QQ==","format":"wav"}}]}]"#,
    expected: r#"[{"role":"system","content":[{"type":"image_url","image_url":"https://example.test/a.png"}]},{"role":"user","content":[{"type":"text","text":"t"},{"type":"input_audio","input_audio":{"data":"QQ==","format":"wav"}}]}]"#,
}))]
#[case::message_order_is_kept(Scenario::Convert(Case {
    messages: r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"role":"user","content":[{"type":"image_url","image_url":"https://example.test/a.png"}]}]"#,
    expected: r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}]"#,
}))]
#[case::text_is_concatenated_across_messages(Scenario::Text(TextCase {
    messages: r#"[{"role":"system","content":"be brief"},{"role":"user","content":"hello"},{"role":"user","content":[{"type":"text","text":"one"},{"type":"image_url","image_url":"https://example.test/a.png"},{"type":"text","text":"two"}]}]"#,
    expected: "be briefhelloonetwo",
}))]
#[case::text_comes_from_every_role(Scenario::Text(TextCase {
    messages: r#"[{"role":"tool","content":[{"type":"text","text":"res"}]},{"role":"assistant","content":"done"}]"#,
    expected: "resdone",
}))]
#[case::missing_and_empty_content_add_no_text(Scenario::Text(TextCase {
    messages: r#"[{"role":"assistant","content":null},{"role":"assistant","content":[]},{"role":"user","content":""}]"#,
    expected: "",
}))]
#[case::valid_json_tool_content_is_kept(Scenario::Stringify(Case {
    messages: r#"[{"role":"tool","tool_call_id":"c1","content":"{\"ok\":true}"},{"role":"tool","tool_call_id":"c2","content":"[1,2]"}]"#,
    expected: r#"[{"role":"tool","tool_call_id":"c1","content":"{\"ok\":true}"},{"role":"tool","tool_call_id":"c2","content":"[1,2]"}]"#,
}))]
#[case::invalid_json_tool_content_is_wrapped(Scenario::Stringify(Case {
    messages: r#"[{"role":"tool","tool_call_id":"c1","content":"plain text"},{"role":"tool","tool_call_id":"c2","content":""},{"role":"tool","tool_call_id":"c3","content":"  "}]"#,
    expected: r#"[{"role":"tool","tool_call_id":"c1","content":"{\"result\":\"plain text\"}"},{"role":"tool","tool_call_id":"c2","content":"{\"result\":\"\"}"},{"role":"tool","tool_call_id":"c3","content":"{\"result\":\"  \"}"}]"#,
}))]
#[case::non_tool_and_list_content_are_untouched(Scenario::Stringify(Case {
    messages: r#"[{"role":"assistant","content":"plain text"},{"role":"tool","tool_call_id":"c1","content":[{"type":"text","text":"plain text"}]}]"#,
    expected: r#"[{"role":"assistant","content":"plain text"},{"role":"tool","tool_call_id":"c1","content":[{"type":"text","text":"plain text"}]}]"#,
}))]
#[case::function_call_must_be_an_object(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"assistant","content":"hi","function_call":"f"}]"#,
    operation: Operation::Convert,
    failure: Failure::Index(IndexKind::FunctionCall, 0),
}))]
#[case::function_arguments_must_be_a_string(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"assistant","content":"hi","function_call":{"name":"f","arguments":5}}]"#,
    operation: Operation::Convert,
    failure: Failure::Index(IndexKind::Arguments, 0),
}))]
#[case::image_url_must_exist(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"user","content":[{"type":"image_url"}]}]"#,
    operation: Operation::Convert,
    failure: Failure::Position(PositionKind::MissingImageUrl, 0, 0),
}))]
#[case::image_url_must_be_a_string_or_object(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"user","content":[{"type":"image_url","image_url":5}]}]"#,
    operation: Operation::Convert,
    failure: Failure::Position(PositionKind::ImageUrl, 0, 0),
}))]
#[case::file_must_be_an_object(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"user","content":[{"type":"file","file":"a.pdf"}]}]"#,
    operation: Operation::Convert,
    failure: Failure::Position(PositionKind::File, 0, 0),
}))]
#[case::null_file_is_refused(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"user","content":[{"type":"file","file":null}]}]"#,
    operation: Operation::Convert,
    failure: Failure::Position(PositionKind::File, 0, 0),
}))]
#[case::failures_name_the_message_and_part(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"user","content":"a"},{"role":"user","content":[{"type":"text","text":"t"},{"type":"file","file":5}]}]"#,
    operation: Operation::Convert,
    failure: Failure::Position(PositionKind::File, 1, 1),
}))]
#[case::text_parts_must_carry_text(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"user","content":[{"type":"text"}]}]"#,
    operation: Operation::Text,
    failure: Failure::Position(PositionKind::Text, 0, 0),
}))]
#[case::tool_messages_must_carry_content(Scenario::Failure(FailureCase {
    messages: r#"[{"role":"tool","tool_call_id":"c1"}]"#,
    operation: Operation::Stringify,
    failure: Failure::ToolContent(0),
}))]
#[case::convert_refuses_too_many_messages(Scenario::MessageLimit(Operation::Convert))]
#[case::text_refuses_too_many_messages(Scenario::MessageLimit(Operation::Text))]
#[case::stringify_refuses_too_many_messages(Scenario::MessageLimit(Operation::Stringify))]
#[case::convert_accepts_the_message_limit(Scenario::MessageAtLimit(Operation::Convert))]
#[case::text_accepts_the_message_limit(Scenario::MessageAtLimit(Operation::Text))]
#[case::stringify_accepts_the_message_limit(Scenario::MessageAtLimit(Operation::Stringify))]
#[case::convert_refuses_too_many_parts(Scenario::PartLimit(Operation::Convert))]
#[case::text_refuses_too_many_parts(Scenario::PartLimit(Operation::Text))]
#[case::convert_accepts_the_part_limit(Scenario::PartsAtLimit(Operation::Convert))]
#[case::text_accepts_the_part_limit(Scenario::PartsAtLimit(Operation::Text))]
#[case::repeated_conversions_do_not_change_the_source(Scenario::Preserved(
    r#"[{"role":"assistant","content":"hi","function_call":{"name":"f"}},{"role":"user","content":[{"type":"image_url","image_url":"https://example.test/a.png"},{"type":"file","file":{"format":"pdf","filename":"a.pdf"}}]}]"#,
))]
fn other_prompts_provider_message_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Convert(case) => assert_messages(&case, to_azure),
        Scenario::Text(case) => assert_text(&case),
        Scenario::Stringify(case) => assert_messages(&case, stringify_tool_content),
        Scenario::Failure(case) => assert_failure(&case),
        Scenario::MessageLimit(operation) => {
            let messages = vec![one_message(); MAX_MESSAGES.saturating_add(1)];
            assert!(matches!(
                run_operation(operation, &messages),
                Err(AzureError::TooManyMessages { limit }) if limit == MAX_MESSAGES
            ));
        }
        Scenario::MessageAtLimit(operation) => {
            let messages = vec![one_message(); MAX_MESSAGES];
            run_operation(operation, &messages).unwrap();
        }
        Scenario::PartLimit(operation) => {
            let messages = vec![parts_message(MAX_MESSAGE_PARTS.saturating_add(1))];
            assert!(matches!(
                run_operation(operation, &messages),
                Err(AzureError::TooManyParts { limit }) if limit == MAX_MESSAGE_PARTS
            ));
        }
        Scenario::PartsAtLimit(operation) => {
            let messages = vec![parts_message(MAX_MESSAGE_PARTS)];
            run_operation(operation, &messages).unwrap();
        }
        Scenario::Preserved(source) => assert_preserved(source),
    }
}

#[rstest]
fn replays_the_provider_messages_corpus(#[files("fuzz/corpus/provider_messages/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(messages) = serde_json::from_slice::<Vec<Message>>(&bytes) else {
        return;
    };
    drop(to_azure(&messages));
    drop(stringify_tool_content(&messages));
    drop(azure_text_pt(&messages));
    drop(cohere_messages_pt_v2(&messages));
    drop(cohere_message_pt(&messages));
}

#![cfg(test)]

use rstest::rstest;

use super::error::BillingTextError;
use super::{
    ChoiceText, EventText, FormattedPromptData, PromptCallType, ResponseText, count_characters,
    get_formatted_prompt, get_response_string,
};
use crate::decisions::Json;

fn json(document: &str) -> Json {
    serde_json::from_str(document).expect("the fixture is JSON")
}

fn json_list(document: &str) -> Vec<Json> {
    let Json::List(messages) = json(document) else {
        panic!("the fixture is a message list");
    };
    messages
}

fn format_chat(document: &str, call_type: PromptCallType) -> Result<String, BillingTextError> {
    let messages = json_list(document);
    get_formatted_prompt(
        &FormattedPromptData {
            messages: Some(&messages),
            prompt: None,
            input: None,
        },
        call_type,
    )
}

fn format_input(document: &str) -> Result<String, BillingTextError> {
    let input = json(document);
    get_formatted_prompt(
        &FormattedPromptData {
            messages: None,
            prompt: None,
            input: Some(&input),
        },
        PromptCallType::Embedding,
    )
}

fn response<'text>(
    event: Option<EventText<'text>>,
    choices: &'text [ChoiceText<'text>],
) -> ResponseText<'text> {
    ResponseText { event, choices }
}

#[rstest]
#[case::empty("", 0)]
#[case::ascii(" a\tb\nc\r", 3)]
#[case::unicode_spaces("\u{a0}a\u{3000}b\u{2003}c", 3)]
#[case::python_separators("\u{1c}\u{1d}\u{1e}\u{1f}", 0)]
#[case::zero_width_space("a\u{200b}b", 3)]
#[case::combining_mark("e\u{301}", 2)]
#[case::emoji("\u{1f600}!", 2)]
fn counts_characters_that_are_not_python_whitespace(#[case] text: &str, #[case] expected: usize) {
    assert_eq!(count_characters(text), expected);
}

#[rstest]
#[case::completion(PromptCallType::Completion)]
#[case::acompletion(PromptCallType::Acompletion)]
fn concatenates_message_text_parts_and_tool_arguments(#[case] call_type: PromptCallType) {
    let parsed = json(
        r#"[
            {"role":"system","content":"sys"},
            {"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"x"}},{"type":"text","text":" there"}]},
            {"role":"assistant","content":null,"tool_calls":[{"function":{"arguments":"{\"a\":1}"}},{"id":"no-function"}]}
        ]"#,
    );
    let Json::List(messages) = parsed else {
        panic!("the fixture is a message list");
    };
    let prompt = get_formatted_prompt(
        &FormattedPromptData {
            messages: Some(&messages),
            prompt: None,
            input: None,
        },
        call_type,
    )
    .expect("a well-formed chat request formats");

    assert_eq!(prompt, "syshi there{\"a\":1}");
}

#[rstest]
#[case::empty_messages("[]", "")]
#[case::null_content(r#"[{"content":null},{"content":"x"}]"#, "x")]
#[case::skipped_function(r#"[{"tool_calls":[{"id":"no-function"}]}]"#, "")]
#[case::empty_tool_calls(r#"[{"content":"x","tool_calls":[]}]"#, "x")]
#[case::null_tool_calls(r#"[{"content":"x","tool_calls":null}]"#, "x")]
fn formats_chat_content_that_is_present(#[case] messages: &str, #[case] expected: &str) {
    assert_eq!(
        format_chat(messages, PromptCallType::Completion).expect("formats"),
        expected
    );
}

#[rstest]
#[case::content_number(r#"[{"content":5}]"#, BillingTextError::NotTextOrList { field: "messages.content" })]
#[case::part_without_type(r#"[{"content":[{"text":"x"}]}]"#, BillingTextError::Missing { field: "messages.content.type" })]
#[case::part_type_not_text(r#"[{"content":[{"type":5}]}]"#, BillingTextError::NotText { field: "messages.content.type" })]
#[case::part_without_text(r#"[{"content":[{"type":"text"}]}]"#, BillingTextError::Missing { field: "messages.content.text" })]
#[case::part_text_not_text(r#"[{"content":[{"type":"text","text":5}]}]"#, BillingTextError::NotText { field: "messages.content.text" })]
#[case::part_not_object(r#"[{"content":["x"]}]"#, BillingTextError::NotObject { field: "messages.content" })]
#[case::message_not_object("[5]", BillingTextError::NotObject { field: "messages" })]
#[case::tool_calls_not_list(r#"[{"tool_calls":{}}]"#, BillingTextError::NotList { field: "messages.tool_calls" })]
#[case::tool_call_not_object(r#"[{"tool_calls":[5]}]"#, BillingTextError::NotObject { field: "messages.tool_calls" })]
#[case::function_not_object(r#"[{"tool_calls":[{"function":null}]}]"#, BillingTextError::NotObject { field: "messages.tool_calls.function" })]
#[case::missing_arguments(r#"[{"tool_calls":[{"function":{}}]}]"#, BillingTextError::Missing { field: "messages.tool_calls.function.arguments" })]
#[case::arguments_not_text(r#"[{"tool_calls":[{"function":{"arguments":5}}]}]"#, BillingTextError::NotText { field: "messages.tool_calls.function.arguments" })]
fn rejects_malformed_chat_fields(#[case] messages: &str, #[case] expected: BillingTextError) {
    assert_eq!(
        format_chat(messages, PromptCallType::Completion),
        Err(expected)
    );
}

#[test]
fn requires_messages_for_a_chat_prompt() {
    assert_eq!(
        get_formatted_prompt(
            &FormattedPromptData {
                messages: None,
                prompt: None,
                input: None,
            },
            PromptCallType::Completion,
        ),
        Err(BillingTextError::Missing { field: "messages" })
    );
}

#[rstest]
#[case::text_completion("text", PromptCallType::TextCompletion)]
#[case::image_generation("image", PromptCallType::ImageGeneration)]
fn reads_the_prompt_field(#[case] document: &str, #[case] call_type: PromptCallType) {
    let prompt = json(&format!("\"{document}\""));
    assert_eq!(
        get_formatted_prompt(
            &FormattedPromptData {
                messages: None,
                prompt: Some(&prompt),
                input: None,
            },
            call_type,
        )
        .expect("formats"),
        document
    );
}

#[rstest]
#[case::number("5", BillingTextError::NotText { field: "prompt" })]
#[case::null("null", BillingTextError::Missing { field: "prompt" })]
fn rejects_malformed_prompt_fields(#[case] document: &str, #[case] expected: BillingTextError) {
    let prompt = json(document);
    assert_eq!(
        get_formatted_prompt(
            &FormattedPromptData {
                messages: None,
                prompt: Some(&prompt),
                input: None,
            },
            PromptCallType::TextCompletion,
        ),
        Err(expected)
    );
}

#[test]
fn requires_the_prompt_field() {
    assert_eq!(
        get_formatted_prompt(
            &FormattedPromptData {
                messages: None,
                prompt: None,
                input: None,
            },
            PromptCallType::ImageGeneration,
        ),
        Err(BillingTextError::Missing { field: "prompt" })
    );
}

#[rstest]
#[case::single_string("\"one\"", "one")]
#[case::list_of_strings(r#"["one","two"]"#, "onetwo")]
#[case::empty_list("[]", "")]
fn concatenates_embedding_input(#[case] document: &str, #[case] expected: &str) {
    assert_eq!(format_input(document).expect("formats"), expected);
}

#[rstest]
#[case::list_item_number("[5]", BillingTextError::NotText { field: "input" })]
#[case::object(r#"{"text":"one"}"#, BillingTextError::NotTextOrList { field: "input" })]
#[case::null("null", BillingTextError::Missing { field: "input" })]
fn rejects_malformed_embedding_input(#[case] document: &str, #[case] expected: BillingTextError) {
    assert_eq!(format_input(document), Err(expected));
}

#[rstest]
#[case::absent(None, "")]
#[case::null(Some("null"), "")]
#[case::text(Some("\"spoken\""), "spoken")]
fn permits_a_missing_transcription_prompt(#[case] document: Option<&str>, #[case] expected: &str) {
    let prompt = document.map(json);
    let formatted = get_formatted_prompt(
        &FormattedPromptData {
            messages: None,
            prompt: prompt.as_ref(),
            input: None,
        },
        PromptCallType::AudioTranscription,
    )
    .expect("formats");

    assert_eq!(formatted, expected);
}

#[test]
fn rejects_a_nontext_transcription_prompt() {
    let prompt = json("5");
    assert_eq!(
        get_formatted_prompt(
            &FormattedPromptData {
                messages: None,
                prompt: Some(&prompt),
                input: None,
            },
            PromptCallType::AudioTranscription,
        ),
        Err(BillingTextError::NotText { field: "prompt" })
    );
}

#[test]
fn response_output_wins_over_delta_and_choices() {
    let output = json_list(r#"[{"content":[{"text":"a"},{"type":"image"}]},{"text":"b"}]"#);
    let delta = json("\"delta\"");
    let choice = json("\"choice\"");
    let choices = [ChoiceText::Message {
        content: Some(&choice),
    }];
    let event = EventText {
        event_type: "response.output_text.delta",
        output: Some(&output),
        delta: Some(&delta),
    };

    assert_eq!(
        get_response_string(&response(Some(event), &choices)).expect("formats"),
        "ab"
    );
}

#[test]
fn an_empty_output_still_returns_no_text() {
    let output = Vec::new();
    let delta = json("\"delta\"");
    let choice = json("\"choice\"");
    let choices = [ChoiceText::Delta {
        content: Some(&choice),
    }];
    let event = EventText {
        event_type: "response.output_text.delta",
        output: Some(&output),
        delta: Some(&delta),
    };

    assert_eq!(
        get_response_string(&response(Some(event), &choices)).expect("formats"),
        ""
    );
}

#[rstest]
#[case::text_delta("response.text.delta")]
#[case::output_text_delta("response.output_text.delta")]
fn a_matching_event_returns_its_delta(#[case] event_type: &str) {
    let delta = json("\"streamed\"");
    let choice = json("\"choice\"");
    let choices = [ChoiceText::Message {
        content: Some(&choice),
    }];
    let event = EventText {
        event_type,
        output: None,
        delta: Some(&delta),
    };

    assert_eq!(
        get_response_string(&response(Some(event), &choices)).expect("formats"),
        "streamed"
    );
}

#[rstest]
#[case::other_event("response.completed", Some("\"delta\""))]
#[case::matching_event_without_a_delta("response.output_text.delta", None)]
fn falls_back_to_choices(#[case] event_type: &str, #[case] delta: Option<&str>) {
    let delta_value = delta.map(json);
    let choice = json("\"choice\"");
    let choices = [ChoiceText::Message {
        content: Some(&choice),
    }];
    let event = EventText {
        event_type,
        output: None,
        delta: delta_value.as_ref(),
    };

    assert_eq!(
        get_response_string(&response(Some(event), &choices)).expect("formats"),
        "choice"
    );
}

#[test]
fn a_null_delta_returns_no_text() {
    let delta = json("null");
    let event = EventText {
        event_type: "response.text.delta",
        output: None,
        delta: Some(&delta),
    };

    assert_eq!(
        get_response_string(&response(Some(event), &[])).expect("formats"),
        ""
    );
}

#[test]
fn concatenates_choice_message_and_delta_content() {
    let message = json("\"one\"");
    let delta = json("\"two\"");
    let choices = [
        ChoiceText::Message {
            content: Some(&message),
        },
        ChoiceText::Delta {
            content: Some(&delta),
        },
        ChoiceText::Message { content: None },
        ChoiceText::Delta { content: None },
    ];

    assert_eq!(
        get_response_string(&response(None, &choices)).expect("formats"),
        "onetwo"
    );
}

#[test]
fn an_empty_choice_list_returns_an_empty_string() {
    assert_eq!(
        get_response_string(&response(None, &[])).expect("formats"),
        ""
    );
}

#[rstest]
#[case::delta_number(Some("5"), None)]
#[case::content_object(None, Some(r#"{"text":"x"}"#))]
fn rejects_nontext_response_content(#[case] delta: Option<&str>, #[case] content: Option<&str>) {
    let delta_value = delta.map(json);
    let content_value = content.map(json);
    let event = EventText {
        event_type: "response.text.delta",
        output: None,
        delta: delta_value.as_ref(),
    };
    let choices = [ChoiceText::Delta {
        content: content_value.as_ref(),
    }];

    get_response_string(&response(Some(event), &choices)).expect_err("rejects");
}

#[test]
fn skips_output_items_without_text() {
    let output = json_list(r#"[5,{"content":[5,{"text":"kept"}]}]"#);

    assert_eq!(
        get_response_string(&response(
            Some(EventText {
                event_type: "response.completed",
                output: Some(&output),
                delta: None,
            }),
            &[],
        ))
        .expect("formats"),
        "kept"
    );
}

#[test]
fn rejects_a_nonlist_output_content() {
    let output = json_list(r#"[{"content":"text"}]"#);

    assert_eq!(
        get_response_string(&response(
            Some(EventText {
                event_type: "response.completed",
                output: Some(&output),
                delta: None,
            }),
            &[],
        )),
        Err(BillingTextError::NotList {
            field: "output.content"
        })
    );
}

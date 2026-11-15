use super::{array_of, body, field_text, integer, list, object, text};
use crate::cato::error::CatoError;
use crate::cato::redaction::anonymize_request;
use crate::cato::wire::{AnalyzeResponse, RedactedChat, RedactedMessage};

fn redacted(entries: Vec<RedactedMessage>) -> AnalyzeResponse {
    AnalyzeResponse {
        required_action: None,
        analysis_result: None,
        redacted_chat: Some(RedactedChat {
            all_redacted_messages: Some(entries),
        }),
    }
}

fn with_content(content: &str) -> RedactedMessage {
    RedactedMessage {
        content: Some(text(content)),
    }
}

fn without_content() -> RedactedMessage {
    RedactedMessage { content: None }
}

#[test]
fn cato_guardrail_anonymize_rewrites_messages_by_index_and_keeps_the_rest() {
    let mut data = body(&[(
        "messages",
        list(&[
            object(&[
                ("content", text("one")),
                ("name", text("kept")),
                ("role", text("user")),
            ]),
            object(&[("content", text("two")), ("role", text("user"))]),
        ]),
    )]);
    let response = redacted(vec![with_content("ONE"), without_content()]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("messages"),
        Some(&list(&[
            object(&[
                ("content", text("ONE")),
                ("name", text("kept")),
                ("role", text("user")),
            ]),
            object(&[("content", text("two")), ("role", text("user"))]),
        ]))
    );
}

#[test]
fn cato_guardrail_anonymize_leaves_messages_past_the_redacted_batch() {
    let mut data = body(&[(
        "messages",
        list(&[
            object(&[("content", text("one"))]),
            object(&[("content", text("two"))]),
        ]),
    )]);
    let response = redacted(vec![with_content("ONE")]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        field_text(&array_of(&data, "messages"), "content"),
        ["ONE", "two"]
    );
}

#[test]
fn cato_guardrail_anonymize_rewrites_messages_and_extra_sources_together() {
    let mut data = body(&[
        (
            "messages",
            list(&[
                object(&[("content", text("one"))]),
                object(&[("content", text("two"))]),
            ]),
        ),
        ("instructions", text("original instructions")),
    ]);
    let response = redacted(vec![
        with_content("ONE"),
        with_content("TWO"),
        with_content("REDACTED INSTRUCTIONS"),
    ]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        field_text(&array_of(&data, "messages"), "content"),
        ["ONE", "TWO"]
    );
    assert_eq!(
        data.get("instructions"),
        Some(&text("REDACTED INSTRUCTIONS"))
    );
}

#[test]
fn cato_guardrail_anonymize_keeps_a_responses_input_when_no_redaction_covers_it() {
    let input = list(&[object(&[
        ("type", text("input_text")),
        ("text", text("original")),
    ])]);
    let mut data = body(&[("input", input.clone())]);
    let response = redacted(vec![]);
    anonymize_request(&response, &mut data).expect("pass through");
    assert_eq!(data.get("input"), Some(&input));
}

#[test]
fn cato_guardrail_anonymize_rewrites_instructions() {
    let mut data = body(&[("instructions", text("original instructions"))]);
    let response = redacted(vec![with_content("REDACTED INSTRUCTIONS")]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("instructions"),
        Some(&text("REDACTED INSTRUCTIONS"))
    );
}

#[test]
fn cato_guardrail_anonymize_keeps_instructions_when_the_redaction_is_null() {
    let mut data = body(&[("instructions", text("original instructions"))]);
    let response = redacted(vec![without_content()]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("instructions"),
        Some(&text("original instructions"))
    );
}

#[test]
fn cato_guardrail_anonymize_rewrites_a_string_prompt() {
    let mut data = body(&[("prompt", text("original prompt"))]);
    let response = redacted(vec![with_content("REDACTED PROMPT")]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(data.get("prompt"), Some(&text("REDACTED PROMPT")));
}

#[test]
fn cato_guardrail_anonymize_rewrites_a_prompt_list_by_position() {
    let mut data = body(&[(
        "prompt",
        list(&[text("first"), text(""), integer(7), text("second")]),
    )]);
    let response = redacted(vec![with_content("FIRST"), with_content("SECOND")]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("prompt"),
        Some(&list(&[
            text("FIRST"),
            text(""),
            integer(7),
            text("SECOND"),
        ]))
    );
}

#[test]
fn cato_guardrail_anonymize_rewrites_schema_strings_in_place() {
    let mut data = body(&[(
        "tools",
        list(&[object(&[(
            "function",
            object(&[
                ("description", text("original description")),
                ("enum", list(&[text("a"), text("b")])),
                ("title", text("original title")),
            ]),
        )])]),
    )]);
    let response = redacted(vec![
        with_content("REDACTED DESCRIPTION"),
        without_content(),
        with_content("REDACTED ENUM ONE"),
        with_content("REDACTED ENUM TWO"),
    ]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("tools"),
        Some(&list(&[object(&[(
            "function",
            object(&[
                ("description", text("REDACTED DESCRIPTION")),
                (
                    "enum",
                    list(&[text("REDACTED ENUM ONE"), text("REDACTED ENUM TWO")])
                ),
                ("title", text("original title")),
            ]),
        )])]))
    );
}

#[test]
fn cato_guardrail_anonymize_rewrites_function_and_response_format_strings() {
    let mut data = body(&[
        (
            "functions",
            list(&[object(&[("description", text("function description"))])]),
        ),
        (
            "response_format",
            object(&[("title", text("format title"))]),
        ),
    ]);
    let response = redacted(vec![
        with_content("REDACTED FUNCTION"),
        with_content("REDACTED FORMAT"),
    ]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("functions"),
        Some(&list(&[object(&[(
            "description",
            text("REDACTED FUNCTION")
        )])]))
    );
    assert_eq!(
        data.get("response_format"),
        Some(&object(&[("title", text("REDACTED FORMAT"))]))
    );
}

#[test]
fn cato_guardrail_anonymize_rewrites_a_string_input() {
    let mut data = body(&[("input", text("original input"))]);
    let response = redacted(vec![with_content("REDACTED INPUT")]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(data.get("input"), Some(&text("REDACTED INPUT")));
}

#[test]
fn cato_guardrail_anonymize_rewrites_a_string_batch_by_non_empty_position() {
    let mut data = body(&[("input", list(&[text("a"), text(""), text("c")]))]);
    let response = redacted(vec![with_content("A"), with_content("C")]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("input"),
        Some(&list(&[text("A"), text(""), text("C")]))
    );
}

#[test]
fn cato_guardrail_anonymize_rejects_a_string_batch_of_the_wrong_size() {
    let mut data = body(&[("input", list(&[text("a"), text("b")]))]);
    let response = redacted(vec![with_content("A")]);
    let error = anonymize_request(&response, &mut data).expect_err("mismatch");
    assert!(matches!(error, CatoError::RedactionMismatch));
    assert_eq!(data.get("input"), Some(&list(&[text("a"), text("b")])));
}

#[test]
fn cato_guardrail_anonymize_rejects_a_string_batch_without_redacted_content() {
    let mut data = body(&[("input", list(&[text("a"), text("b")]))]);
    let response = redacted(vec![without_content(), without_content()]);
    let error = anonymize_request(&response, &mut data).expect_err("mismatch");
    assert!(matches!(error, CatoError::RedactionMismatch));
}

#[test]
fn cato_guardrail_anonymize_passes_through_without_a_redacted_chat() {
    let mut data = body(&[("messages", list(&[object(&[("content", text("hi"))])]))]);
    let original = data.clone();
    let response = AnalyzeResponse {
        required_action: None,
        analysis_result: None,
        redacted_chat: None,
    };
    anonymize_request(&response, &mut data).expect("pass through");
    assert_eq!(data, original);
}

#[test]
fn cato_guardrail_anonymize_passes_through_with_an_empty_redacted_chat() {
    let mut data = body(&[("messages", list(&[object(&[("content", text("hi"))])]))]);
    let original = data.clone();
    let response = AnalyzeResponse {
        required_action: None,
        analysis_result: None,
        redacted_chat: Some(RedactedChat::default()),
    };
    anonymize_request(&response, &mut data).expect("pass through");
    assert_eq!(data, original);
}

#[test]
fn cato_guardrail_anonymize_applies_a_prompt_list_batch_that_stops_early() {
    let mut data = body(&[("prompt", list(&[text("a"), text("b"), text("c")]))]);
    let response = redacted(vec![with_content("A"), with_content("B")]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("prompt"),
        Some(&list(&[text("A"), text("B"), text("c")]))
    );
}

#[test]
fn cato_guardrail_anonymize_applies_a_schema_batch_that_stops_early() {
    let mut data = body(&[(
        "tools",
        list(&[object(&[(
            "function",
            object(&[
                ("description", text("original description")),
                ("title", text("original title")),
                ("enum", list(&[text("a"), text("b")])),
            ]),
        )])]),
    )]);
    let response = redacted(vec![
        with_content("REDACTED DESCRIPTION"),
        with_content("REDACTED TITLE"),
    ]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(
        data.get("tools"),
        Some(&list(&[object(&[(
            "function",
            object(&[
                ("description", text("REDACTED DESCRIPTION")),
                ("enum", list(&[text("a"), text("b")])),
                ("title", text("REDACTED TITLE")),
            ]),
        )])]))
    );
}

#[test]
fn cato_guardrail_anonymize_rejects_a_string_batch_with_an_empty_redacted_chat() {
    let mut data = body(&[("input", list(&[text("a")]))]);
    let response = AnalyzeResponse {
        required_action: None,
        analysis_result: None,
        redacted_chat: Some(RedactedChat::default()),
    };
    let error = anonymize_request(&response, &mut data).expect_err("empty batch");
    assert!(matches!(error, CatoError::RedactionMismatch));
}

#[test]
fn cato_guardrail_anonymize_leaves_a_responses_input_list_unchanged() {
    let input = list(&[object(&[
        ("type", text("input_text")),
        ("text", text("original")),
    ])]);
    let mut data = body(&[("input", input.clone())]);
    let response = redacted(vec![with_content("REDACTED")]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(data.get("input"), Some(&input));
}

#[test]
fn cato_guardrail_anonymize_ignores_redactions_without_a_content_field() {
    let mut data = body(&[("input", text("original"))]);
    let response = redacted(vec![without_content()]);
    anonymize_request(&response, &mut data).expect("rewritten");
    assert_eq!(data.get("input"), Some(&text("original")));
}

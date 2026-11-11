use betterlite_openai::chat::{MessageContent, Role};
use serde_json::json;

use super::prelude::{text_of, translate};

#[test]
fn the_first_system_part_becomes_the_system_message() {
    let request = translate(json!({
        "model": "m",
        "systemInstruction": {"parts": [{"text": "be terse"}, {"text": "ignored"}]},
        "contents": [{"parts": [{"text": "hi"}]}],
    }));
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.messages[0].role, Role::System);
    assert_eq!(text_of(&request.messages[0]), "be terse");
    assert_eq!(request.messages[1].role, Role::User);
}

#[test]
fn a_null_system_parts_list_drops_the_instruction() {
    let request = translate(json!({
        "model": "m",
        "systemInstruction": {"parts": null},
        "contents": [{"parts": [{"text": "hi"}]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::User);
}

#[test]
fn a_system_instruction_without_parts_is_dropped() {
    let request = translate(json!({
        "model": "m",
        "systemInstruction": {},
        "contents": [{"parts": [{"text": "hi"}]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::User);
}

#[test]
fn a_first_system_part_without_text_drops_the_whole_instruction() {
    let request = translate(json!({
        "model": "m",
        "systemInstruction": {"parts": [{"inline_data": {"data": "AA"}}, {"text": "ignored"}]},
        "contents": [{"parts": [{"text": "hi"}]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::User);
}

#[test]
fn an_empty_system_text_keeps_an_empty_system_message() {
    let request = translate(json!({
        "model": "m",
        "systemInstruction": {"parts": [{"text": ""}]},
        "contents": [],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(
        request.messages[0].content,
        Some(MessageContent::Text(String::new()))
    );
}

#[test]
fn a_null_system_instruction_is_dropped() {
    let request = translate(json!({
        "model": "m",
        "systemInstruction": null,
        "contents": [{"parts": [{"text": "hi"}]}],
    }));
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, Role::User);
}

#[test]
fn the_snake_case_system_instruction_is_read() {
    let request = translate(json!({
        "model": "m",
        "system_instruction": {"parts": [{"text": "be terse"}]},
        "contents": [{"parts": [{"text": "hi"}]}],
    }));
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.messages[0].role, Role::System);
    assert_eq!(text_of(&request.messages[0]), "be terse");
}

#[test]
fn the_camel_case_system_instruction_wins_over_the_snake_case_one() {
    let request = translate(json!({
        "model": "m",
        "systemInstruction": {"parts": [{"text": "camel"}]},
        "system_instruction": {"parts": [{"text": "snake"}]},
        "contents": [],
    }));
    assert_eq!(text_of(&request.messages[0]), "camel");
}

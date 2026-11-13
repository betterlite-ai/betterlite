#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;

use crate::system_instruction::{SystemInstruction, TextPart, extract_system_instruction};

fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn encode<T>(value: &T) -> Json
where
    T: serde::Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

#[test]
fn extracts_system_messages_in_order() {
    let parsed = messages(
        r#"[{"role":"user","content":"q"},{"role":"system","content":"first"},{"role":"assistant","content":"a"},{"role":"system","content":[{"type":"text","text":"head"},{"type":"text","text":"tail"}]}]"#,
    );
    let extraction = extract_system_instruction(true, &parsed);
    assert_eq!(
        extraction.instruction,
        Some(SystemInstruction {
            parts: vec![
                TextPart {
                    text: "first".to_owned()
                },
                TextPart {
                    text: "headtail".to_owned()
                }
            ],
        })
    );
    assert_eq!(
        encode(&extraction.messages),
        json(r#"[{"role":"user","content":"q"},{"role":"assistant","content":"a"}]"#)
    );
}

#[test]
fn keeps_system_messages_when_the_model_does_not_support_them() {
    let parsed = messages(r#"[{"role":"system","content":"rules"},{"role":"user","content":"q"}]"#);
    let extraction = extract_system_instruction(false, &parsed);
    assert_eq!(extraction.instruction, None);
    assert_eq!(encode(&extraction.messages), encode(&parsed));
}

#[test]
fn keeps_a_system_message_without_content() {
    let parsed = messages(r#"[{"role":"system"},{"role":"user","content":"q"}]"#);
    let extraction = extract_system_instruction(true, &parsed);
    assert_eq!(extraction.instruction, None);
    assert_eq!(encode(&extraction.messages), encode(&parsed));
}

#[test]
fn a_system_only_request_gains_a_user_period() {
    let parsed = messages(r#"[{"role":"system","content":"rules"}]"#);
    let extraction = extract_system_instruction(true, &parsed);
    assert_eq!(
        extraction.instruction,
        Some(SystemInstruction {
            parts: vec![TextPart {
                text: "rules".to_owned()
            }],
        })
    );
    assert_eq!(
        encode(&extraction.messages),
        json(r#"[{"role":"user","content":"."}]"#)
    );
}

#[test]
fn an_empty_system_part_list_becomes_empty_text() {
    let parsed = messages(r#"[{"role":"system","content":[]}]"#);
    let extraction = extract_system_instruction(true, &parsed);
    assert_eq!(
        extraction.instruction,
        Some(SystemInstruction {
            parts: vec![TextPart {
                text: String::new()
            }],
        })
    );
    assert_eq!(
        encode(&extraction.messages),
        json(r#"[{"role":"user","content":"."}]"#)
    );
}

#[test]
fn non_text_system_parts_are_ignored() {
    let parsed = messages(
        r#"[{"role":"system","content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"u"}},{"type":"text","text":"b"}]}]"#,
    );
    let extraction = extract_system_instruction(true, &parsed);
    assert_eq!(
        extraction.instruction,
        Some(SystemInstruction {
            parts: vec![TextPart {
                text: "ab".to_owned()
            }],
        })
    );
}

#![cfg(test)]

use super::{SemanticCacheRecordsError, decode_cached_response, extract_semantic_prompt};
use crate::arguments::json::Json;
use crate::message_content::{Content, Message};
use indexmap::IndexMap;
use std::fs;
use std::path::PathBuf;

fn message(text: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: IndexMap::new(),
    }
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

#[test]
fn prompt_messages_take_precedence_over_responses() {
    let messages = [message("chat")];
    let input = json("\"responses\"");
    assert_eq!(
        extract_semantic_prompt(Some(&messages), Some(&input)).unwrap(),
        Some("chat".to_owned())
    );
}

#[test]
fn prompt_empty_message_presence_wins_and_stays_empty() {
    let messages: [Message; 0] = [];
    let input = json("\"responses\"");
    assert_eq!(
        extract_semantic_prompt(Some(&messages), Some(&input)).unwrap(),
        Some(String::new())
    );
}

#[test]
fn prompt_responses_string_input_is_trimmed() {
    let input = json("\"  hello  \"");
    assert_eq!(
        extract_semantic_prompt(None, Some(&input)).unwrap(),
        Some("hello".to_owned())
    );
}

#[test]
fn prompt_null_content_falls_through_to_text_key() {
    let input = json(r#"{"content":null,"text":"fallback"}"#);
    assert_eq!(
        extract_semantic_prompt(None, Some(&input)).unwrap(),
        Some("fallback".to_owned())
    );
}

#[test]
fn prompt_content_precedes_text_key() {
    let input = json(r#"{"content":[{"text":"inside"}],"text":"outside"}"#);
    assert_eq!(
        extract_semantic_prompt(None, Some(&input)).unwrap(),
        Some("inside".to_owned())
    );
}

#[test]
fn prompt_text_keys_use_declared_order() {
    let input =
        json(r#"{"text":" first ","output":"second","input_text":"third","output_text":"fourth"}"#);
    assert_eq!(
        extract_semantic_prompt(None, Some(&input)).unwrap(),
        Some("first".to_owned())
    );
}

#[test]
fn prompt_nested_text_is_trimmed_and_joined_with_newlines() {
    let input = json(r#"[" first ",["second",4,{"output_text":" third "}]]"#);
    assert_eq!(
        extract_semantic_prompt(None, Some(&input)).unwrap(),
        Some("first\nsecond\nthird".to_owned())
    );
}

#[test]
fn prompt_without_extracted_text_is_absent() {
    let input = json(r#"[{"type":"image"},null,false,{"content":null}]"#);
    assert_eq!(extract_semantic_prompt(None, Some(&input)).unwrap(), None);
}

#[test]
fn prompt_chat_projection_preserves_order_and_concatenates_directly() {
    let messages = [message("first"), message("second")];
    assert_eq!(
        extract_semantic_prompt(Some(&messages), None).unwrap(),
        Some("firstsecond".to_owned())
    );
}

#[test]
fn prompt_depth_64_is_accepted() {
    let nested = format!("{}\"deep\"{}", "[".repeat(64), "]".repeat(64));
    let value = json(&nested);
    assert_eq!(
        extract_semantic_prompt(None, Some(&value)).unwrap(),
        Some("deep".to_owned())
    );
}

#[test]
fn prompt_depth_65_is_rejected() {
    let nested = format!("{}\"deep\"{}", "[".repeat(65), "]".repeat(65));
    let value = json(&nested);
    assert!(matches!(
        extract_semantic_prompt(None, Some(&value)),
        Err(SemanticCacheRecordsError::ResponsesDepthExceeded)
    ));
}

#[test]
fn cached_response_absent_input_stays_absent() {
    assert_eq!(decode_cached_response(None).unwrap(), None);
}

#[test]
fn cached_response_json_null_stays_present() {
    assert_eq!(
        decode_cached_response(Some(b"null")).unwrap(),
        Some(Json::Null)
    );
}

#[test]
fn cached_response_malformed_utf8_is_typed_error() {
    assert!(matches!(
        decode_cached_response(Some(&[0xff])),
        Err(SemanticCacheRecordsError::Json(_))
    ));
}

#[test]
fn cached_response_malformed_json_is_typed_error() {
    assert!(matches!(
        decode_cached_response(Some(b"{")),
        Err(SemanticCacheRecordsError::Json(_))
    ));
}

#[test]
fn cached_response_rejects_python_literals() {
    assert!(matches!(
        decode_cached_response(Some(b"{'ok': True}")),
        Err(SemanticCacheRecordsError::Json(_))
    ));
}

#[test]
fn cached_response_rejects_payload_over_one_mebibyte() {
    let payload = vec![b' '; 0x0010_0001];
    assert!(matches!(
        decode_cached_response(Some(&payload)),
        Err(SemanticCacheRecordsError::PayloadTooLarge {
            actual: 0x0010_0001,
            maximum: 0x0010_0000
        })
    ));
}

#[rstest::rstest]
fn replays_the_semantic_cache_response_corpus(
    #[files("fuzz/corpus/semantic_cache_response/*")] path: PathBuf,
) {
    let payload = fs::read(path).unwrap();
    drop(decode_cached_response(Some(&payload)));
}

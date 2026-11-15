#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{
    extract_text_from_content, extract_text_from_message, merge_metadata_bags, normalize_content,
};
use crate::crowdstrike_aidr::error::CrowdStrikeError;
use crate::crowdstrike_aidr::models::{
    ContentPart, ImageUrl, ImageUrlContentPart, ImageUrlPartType, Message, MessageContent,
    TextContentPart, TextPartType,
};

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn object(fields: Vec<(&str, GuardrailValue)>) -> GuardrailValue {
    GuardrailValue::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn text_part(value: &str) -> ContentPart {
    ContentPart::Text(TextContentPart {
        part_type: TextPartType::Text,
        text: value.to_owned(),
    })
}

#[test]
fn _normalize_content_string() {
    assert_eq!(
        normalize_content(&text("hello")).unwrap(),
        Some(MessageContent::Text("hello".to_owned()))
    );
    assert_eq!(normalize_content(&GuardrailValue::Null).unwrap(), None);
    assert_eq!(
        normalize_content(&GuardrailValue::Boolean(true)).unwrap(),
        Some(MessageContent::Text("true".to_owned()))
    );
    assert_eq!(
        normalize_content(&GuardrailValue::SignedInteger(42)).unwrap(),
        Some(MessageContent::Text("42".to_owned()))
    );
}

#[test]
fn _normalize_content_list() {
    let raw = GuardrailValue::Array(vec![
        object(vec![("type", text("text")), ("text", text("first"))]),
        object(vec![
            ("type", text("image_url")),
            (
                "image_url",
                object(vec![("url", text("https://img.test/x.png"))]),
            ),
        ]),
        GuardrailValue::SignedInteger(7),
    ]);
    let normalized = normalize_content(&raw).unwrap().unwrap();
    let expected = vec![
        text_part("first"),
        ContentPart::ImageUrl(ImageUrlContentPart {
            part_type: ImageUrlPartType::ImageUrl,
            image_url: ImageUrl {
                url: "https://img.test/x.png".to_owned(),
            },
        }),
        text_part("7"),
    ];
    assert_eq!(normalized, MessageContent::Parts(expected));
}

#[test]
fn _normalize_content_list_keeps_a_plain_string_image_url() {
    let raw = GuardrailValue::Array(vec![object(vec![
        ("type", text("image_url")),
        ("image_url", text("https://img.test/direct.png")),
    ])]);
    assert_eq!(
        normalize_content(&raw).unwrap(),
        Some(MessageContent::Parts(vec![ContentPart::ImageUrl(
            ImageUrlContentPart {
                part_type: ImageUrlPartType::ImageUrl,
                image_url: ImageUrl {
                    url: "https://img.test/direct.png".to_owned(),
                },
            }
        )]))
    );
}

#[test]
fn _normalize_content_list_drops_unrecognized_types() {
    let raw = GuardrailValue::Array(vec![
        object(vec![
            ("type", text("video_url")),
            ("video_url", object(vec![])),
        ]),
        object(vec![("type", text("text")), ("text", text("kept"))]),
    ]);
    assert_eq!(
        normalize_content(&raw).unwrap(),
        Some(MessageContent::Parts(vec![text_part("kept")]))
    );
}

#[test]
fn _normalize_content_list_drops_a_text_part_whose_text_is_not_a_string() {
    let raw = GuardrailValue::Array(vec![object(vec![
        ("type", text("text")),
        ("text", GuardrailValue::SignedInteger(1)),
    ])]);
    assert_eq!(
        normalize_content(&raw).unwrap(),
        Some(MessageContent::Parts(Vec::new()))
    );
}

#[test]
fn _normalize_content_returns_empty_list_unchanged() {
    assert_eq!(
        normalize_content(&GuardrailValue::Array(Vec::new())).unwrap(),
        Some(MessageContent::Parts(Vec::new()))
    );
    assert_eq!(
        normalize_content(&object(vec![("a", text("b"))])).unwrap(),
        Some(MessageContent::Text("{\"a\":\"b\"}".to_owned()))
    );
    assert_eq!(
        normalize_content(&text("plain")).unwrap(),
        Some(MessageContent::Text("plain".to_owned()))
    );
}

#[test]
fn _normalize_content_rejects_a_wrong_typed_image_url() {
    let raw = GuardrailValue::Array(vec![object(vec![
        ("type", text("image_url")),
        ("image_url", GuardrailValue::SignedInteger(3)),
    ])]);
    assert!(matches!(
        normalize_content(&raw),
        Err(CrowdStrikeError::InvalidParam { name: "image_url" })
    ));
}

#[test]
fn _extract_text_from_content_string() {
    assert_eq!(extract_text_from_content(&text("hello")), "hello");
    assert_eq!(extract_text_from_content(&GuardrailValue::Null), "");
    assert_eq!(
        extract_text_from_content(&GuardrailValue::Boolean(false)),
        ""
    );
    let list = GuardrailValue::Array(vec![
        object(vec![("type", text("text")), ("text", text("a"))]),
        object(vec![("type", text("image_url"))]),
        object(vec![("type", text("text")), ("text", text("b"))]),
    ]);
    assert_eq!(extract_text_from_content(&list), "a\nb");
}

#[test]
fn _extract_text_from_message_string_content() {
    let string_message = Message {
        role: "user".to_owned(),
        content: Some(MessageContent::Text("hello".to_owned())),
    };
    assert_eq!(extract_text_from_message(&string_message), "hello");
    let empty_message = Message {
        role: "user".to_owned(),
        content: None,
    };
    assert_eq!(extract_text_from_message(&empty_message), "");
    let parts_message = Message {
        role: "user".to_owned(),
        content: Some(MessageContent::Parts(vec![
            text_part("a"),
            ContentPart::ImageUrl(ImageUrlContentPart {
                part_type: ImageUrlPartType::ImageUrl,
                image_url: ImageUrl {
                    url: "https://img.test".to_owned(),
                },
            }),
            text_part("b"),
        ])),
    };
    assert_eq!(extract_text_from_message(&parts_message), "a\nb");
}

#[test]
fn _merge_metadata_bags() {
    let both = BTreeMap::from([
        (
            "metadata".to_owned(),
            object(vec![("user_api_key_user_id", text("meta"))]),
        ),
        (
            "litellm_metadata".to_owned(),
            object(vec![
                ("user_api_key_user_id", text("llm")),
                ("user_api_key_user_email", text("llm@example.com")),
            ]),
        ),
    ]);
    let merged = merge_metadata_bags(&both).expect("both bags present");
    assert_eq!(merged.get("user_api_key_user_id"), Some(&text("llm")));
    assert_eq!(
        merged.get("user_api_key_user_email"),
        Some(&text("llm@example.com"))
    );
    let empty = BTreeMap::from([("metadata".to_owned(), object(Vec::new()))]);
    assert_eq!(
        merge_metadata_bags(&empty),
        Some(BTreeMap::<String, GuardrailValue>::new())
    );
    assert_eq!(merge_metadata_bags(&BTreeMap::new()), None);
    let not_a_bag = BTreeMap::from([("metadata".to_owned(), text("nope"))]);
    assert_eq!(merge_metadata_bags(&not_a_bag), None);
}

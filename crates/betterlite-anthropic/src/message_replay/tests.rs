#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};

use crate::message_replay::advisor::strip_advisor_blocks;
use crate::message_replay::cache_control::normalize_cache_control;
use crate::message_replay::provider_fields::strip_provider_specific_fields;
use crate::message_replay::tool_ids::sanitize_tool_use_ids;
use crate::message_replay::{
    BlockStrip, EmptyKind, is_empty_block, is_empty_unsigned_thinking, is_invalid_thinking_error,
    strip_blocks, strip_thinking_from_request,
};

mod advisor_cases;
mod cache_control_cases;
mod case_fixtures;
mod predicate_cases;
mod provider_field_cases;
mod strip_cases;
mod tool_id_cases;

#[rstest::rstest]
#[case::advisor_block_removal(case_fixtures::ADVISOR_INPUT, case_fixtures::ADVISOR_EXPECTED)]
#[case::invalid_thinking_error_match(case_fixtures::INVALID_INPUT, case_fixtures::INVALID_EXPECTED)]
#[case::encrypted_reasoning_removal(
    case_fixtures::ENCRYPTED_INPUT,
    case_fixtures::ENCRYPTED_EXPECTED
)]
#[case::empty_thinking_signed_versus_unsigned(
    case_fixtures::EMPTY_THINKING_INPUT,
    case_fixtures::EMPTY_THINKING_EXPECTED
)]
#[case::empty_text_content(case_fixtures::EMPTY_TEXT_INPUT, case_fixtures::EMPTY_TEXT_EXPECTED)]
#[case::tool_use_id_rewrite(case_fixtures::TOOL_USE_INPUT, case_fixtures::TOOL_USE_EXPECTED)]
#[case::strip_provider_specific_fields(
    case_fixtures::PROVIDER_INPUT,
    case_fixtures::PROVIDER_EXPECTED
)]
#[case::portable_cache_control(case_fixtures::CACHE_INPUT, case_fixtures::CACHE_EXPECTED)]
fn message_replay_cases(#[case] input: &str, #[case] expected: &str) {
    let parsed_input: Json = serde_json::from_str(input).unwrap();
    let parsed_expected: Json = serde_json::from_str(expected).unwrap();
    assert_eq!(replay(&parsed_input), parsed_expected);
}

fn replay(input: &Json) -> Json {
    let fields = input.as_object().unwrap();
    match fields.get("function").and_then(Json::as_str).unwrap() {
        "advisor" => replay_advisor(fields),
        "invalid_thinking_error" => replay_thinking_error(fields),
        "strip_encrypted" => Json::List(strip_blocks(
            &list_field(fields, "messages"),
            BlockStrip::EncryptedReasoning,
        )),
        "empty_predicates" => replay_empty_predicates(fields),
        "strip_empty_content" => Json::List(strip_blocks(
            &list_field(fields, "messages"),
            BlockStrip::EmptyContent,
        )),
        "tool_use_ids" => Json::List(sanitize_tool_use_ids(&list_field(fields, "messages"))),
        "provider_fields" => Json::List(strip_provider_specific_fields(&list_field(
            fields, "messages",
        ))),
        "cache_control" => replay_cache_control(fields),
        other => panic!("unknown replay function {other}"),
    }
}

fn replay_advisor(fields: &Object) -> Json {
    let mut messages = list_field(fields, "messages");
    let replace = matches!(fields.get("replace_with_text"), Some(Json::Bool(true)));
    strip_advisor_blocks(&mut messages, replace);
    Json::List(messages)
}

fn replay_thinking_error(fields: &Object) -> Json {
    let texts = list_field(fields, "texts");
    let matched: Vec<bool> = texts
        .iter()
        .map(|text| is_invalid_thinking_error(text.as_str().unwrap()))
        .collect();
    let requests = list_field(fields, "requests");
    let repaired = requests
        .iter()
        .map(|request| {
            let mut payload = request.as_object().unwrap().clone();
            strip_thinking_from_request(&mut payload);
            Json::Object(payload)
        })
        .collect();
    let mut result = Object::new();
    result.insert("matches".to_owned(), bool_list(matched));
    result.insert("repaired".to_owned(), Json::List(repaired));
    Json::Object(result)
}

fn replay_empty_predicates(fields: &Object) -> Json {
    let blocks = list_field(fields, "blocks");
    let thinking: Vec<bool> = blocks
        .iter()
        .map(|block| is_empty_block(block, EmptyKind::Thinking))
        .collect();
    let unsigned: Vec<bool> = blocks.iter().map(is_empty_unsigned_thinking).collect();
    let text: Vec<bool> = blocks
        .iter()
        .map(|block| is_empty_block(block, EmptyKind::Text))
        .collect();
    Json::Object(Object::from_iter([
        ("thinking".to_owned(), bool_list(thinking)),
        ("unsigned".to_owned(), bool_list(unsigned)),
        ("text".to_owned(), bool_list(text)),
    ]))
}

fn bool_list(values: Vec<bool>) -> Json {
    Json::List(values.into_iter().map(Json::Bool).collect())
}

fn replay_cache_control(fields: &Object) -> Json {
    let payload = fields.get("payload").and_then(Json::as_object).unwrap();
    Json::Object(normalize_cache_control(payload))
}

fn list_field(fields: &Object, key: &str) -> Vec<Json> {
    match fields.get(key) {
        Some(Json::List(items)) => items.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => panic!("the {key} fixture must be a list"),
    }
}

fn list_fixture(text: &str) -> Vec<Json> {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            panic!("the fixture must be a list")
        }
    }
}

fn object_fixture(text: &str) -> Object {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture must be an object")
        }
    }
}

fn json_fixture(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

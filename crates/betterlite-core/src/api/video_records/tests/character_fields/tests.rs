#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::video_edits::response::CharacterKind;
use rstest::rstest;
use serde_json::Number;

use crate::api::video_records::CharacterObject;
use crate::api::video_records::dict_access::DictAccess as _;

use super::fixture::{assert_unknown_field, character, object};

#[test]
fn character_object_reads_the_declared_fields() {
    let character = character("char_123");
    assert_eq!(character.get("id"), Some(Json::Text("char_123".to_owned())));
    assert_eq!(
        character.get("object"),
        Some(Json::Text("character".to_owned()))
    );
    assert_eq!(
        character.get("created_at"),
        Some(Json::Number(Number::from(1_700_000_000_i64)))
    );
    assert_eq!(character.get("name"), Some(Json::Text("Ada".to_owned())));
}

#[test]
fn character_object_defaults_its_object_kind() {
    let decoded: CharacterObject =
        serde_json::from_str(r#"{"id": "char_123", "created_at": 1700000000, "name": "Ada"}"#)
            .unwrap();
    assert_eq!(decoded.object, CharacterKind::Character);
    assert_eq!(
        decoded.get("object"),
        Some(Json::Text("character".to_owned()))
    );
}

#[test]
fn character_object_hides_its_hidden_params() {
    let mut character = character("char_123");
    character.hidden_params = object(&[("team", Json::Text("core".to_owned()))]);
    assert!(character.contains("_hidden_params"));
    assert_eq!(
        character.get("_hidden_params"),
        Some(Json::Object(object(&[(
            "team",
            Json::Text("core".to_owned())
        )])))
    );
    assert!(!character.json().contains_key("_hidden_params"));
    assert_eq!(character.json().len(), 4);
}

#[rstest]
#[case::id("id")]
#[case::object("object")]
#[case::created_at("created_at")]
#[case::name("name")]
fn character_contains_reports_the_declared_fields(#[case] field: &str) {
    let character = character("char_123");
    assert!(character.contains(field), "missing {field}");
}

#[test]
fn character_contains_rejects_an_undeclared_field() {
    let character = character("char_123");
    assert!(!character.contains("prompt"));
    assert!(!character.contains(""));
}

#[test]
fn character_getitem_returns_a_field() {
    assert_eq!(
        character("char_123").getitem("name"),
        Ok(Json::Text("Ada".to_owned()))
    );
}

#[test]
fn character_getitem_refuses_an_unknown_field() {
    let error = character("char_123").getitem("prompt").unwrap_err();
    assert_unknown_field(error, "CharacterObject", "prompt");
}

#[test]
fn character_get_returns_none_for_an_unknown_field() {
    assert_eq!(character("char_123").get("_hidden_params_other"), None);
}

#[test]
fn character_object_serializes_only_declared_fields() {
    let mut character = character("char_123");
    character.hidden_params = object(&[("team", Json::Text("core".to_owned()))]);
    let dumped: Json = serde_json::from_value(serde_json::to_value(&character).unwrap()).unwrap();
    let expected = Json::Object(object(&[
        ("id", Json::Text("char_123".to_owned())),
        ("object", Json::Text("character".to_owned())),
        ("created_at", Json::Number(Number::from(1_700_000_000_i64))),
        ("name", Json::Text("Ada".to_owned())),
    ]));
    assert_eq!(dumped, expected);
}

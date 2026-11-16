#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::{decode_character_id, encode_character_id};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::video_records::helpers::{CharacterResponse, encode_character_id_in_response};

use super::fixture::{assert_not_text, character, object};

fn encoded_id(fields: &IndexMap<String, Json>) -> String {
    match fields.get("id") {
        Some(Json::Text(value)) => value.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => panic!("the response id is not text"),
    }
}

#[test]
fn encode_character_encodes_a_dict_id() {
    let mut fields = object(&[
        ("id", Json::Text("char_abc".to_owned())),
        ("object", Json::Text("character".to_owned())),
    ]);
    encode_character_id_in_response(
        CharacterResponse::Fields(&mut fields),
        "openai",
        Some("sora-2"),
    )
    .unwrap();
    let encoded = encoded_id(&fields);
    assert!(encoded.starts_with("character_"));
    let decoded = decode_character_id(&encoded);
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(decoded.model_id.as_deref(), Some("sora-2"));
    assert_eq!(decoded.character_id, "char_abc");
    assert_eq!(
        fields.get("object"),
        Some(&Json::Text("character".to_owned()))
    );
}

#[test]
fn encode_character_encodes_an_object_id() {
    let mut character = character("char_abc");
    encode_character_id_in_response(CharacterResponse::Object(&mut character), "openai", None)
        .unwrap();
    assert!(character.id.starts_with("character_"));
    let decoded = decode_character_id(&character.id);
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(decoded.model_id.as_deref(), Some(""));
    assert_eq!(decoded.character_id, "char_abc");
}

#[test]
fn encode_character_leaves_an_absent_dict_id() {
    let mut fields = object(&[("object", Json::Text("character".to_owned()))]);
    encode_character_id_in_response(
        CharacterResponse::Fields(&mut fields),
        "openai",
        Some("sora-2"),
    )
    .unwrap();
    assert_eq!(
        fields,
        object(&[("object", Json::Text("character".to_owned()))])
    );
}

#[test]
fn encode_character_leaves_a_null_dict_id() {
    let mut fields = object(&[("id", Json::Null)]);
    encode_character_id_in_response(CharacterResponse::Fields(&mut fields), "openai", None)
        .unwrap();
    assert_eq!(fields.get("id"), Some(&Json::Null));
}

#[test]
fn encode_character_leaves_an_empty_dict_id() {
    let mut fields = object(&[("id", Json::Text(String::new()))]);
    encode_character_id_in_response(CharacterResponse::Fields(&mut fields), "openai", None)
        .unwrap();
    assert_eq!(fields.get("id"), Some(&Json::Text(String::new())));
}

#[test]
fn encode_character_leaves_an_empty_object_id() {
    let mut character = character("");
    encode_character_id_in_response(CharacterResponse::Object(&mut character), "openai", None)
        .unwrap();
    assert_eq!(character.id, "");
}

#[test]
fn encode_character_leaves_an_already_encoded_id() {
    let already = encode_character_id("char_abc", "openai", Some("sora-2"));
    let mut character = character(&already);
    encode_character_id_in_response(
        CharacterResponse::Object(&mut character),
        "anthropic",
        Some("other"),
    )
    .unwrap();
    assert_eq!(character.id, already);
}

#[test]
fn encode_character_leaves_ids_without_a_provider() {
    let mut character = character("char_abc");
    encode_character_id_in_response(
        CharacterResponse::Object(&mut character),
        "",
        Some("sora-2"),
    )
    .unwrap();
    assert_eq!(character.id, "char_abc");
    let mut fields = object(&[("id", Json::Text("char_abc".to_owned()))]);
    encode_character_id_in_response(CharacterResponse::Fields(&mut fields), "", None).unwrap();
    assert_eq!(fields.get("id"), Some(&Json::Text("char_abc".to_owned())));
}

#[rstest]
#[case::bool(Json::Bool(true))]
#[case::number(Json::Number(Number::from(5_i64)))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn encode_character_rejects_a_non_text_dict_id(#[case] value: Json) {
    let mut fields = object(&[("id", value)]);
    let error =
        encode_character_id_in_response(CharacterResponse::Fields(&mut fields), "openai", None)
            .unwrap_err();
    assert_not_text(error, "id");
}

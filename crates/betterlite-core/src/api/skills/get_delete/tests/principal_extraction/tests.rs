#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::api::skills::fixtures::{metadata_with, object, text};
use crate::api::skills::get_delete::principal::user_api_key_auth_from_kwargs;

#[test]
fn principal_extraction_prefers_metadata_over_litellm_metadata() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), metadata_with(text("from-metadata")));
    values.insert(
        "litellm_metadata".to_owned(),
        metadata_with(text("from-litellm-metadata")),
    );
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&text("from-metadata"))
    );
}

#[test]
fn principal_extraction_falls_back_to_litellm_metadata() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), object(&[]));
    values.insert(
        "litellm_metadata".to_owned(),
        metadata_with(text("principal")),
    );
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&text("principal"))
    );
}

#[rstest]
#[case::text(text("principal"))]
#[case::empty_object(object(&[]))]
#[case::false_value(Json::Bool(false))]
#[case::empty_list(Json::List(Vec::new()))]
fn user_api_key_auth_accepts_any_non_null_value(#[case] principal: Json) {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), metadata_with(principal.clone()));
    assert_eq!(user_api_key_auth_from_kwargs(&values), Some(&principal));
}

#[test]
fn user_api_key_auth_null_falls_through_to_litellm_metadata() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), metadata_with(Json::Null));
    values.insert(
        "litellm_metadata".to_owned(),
        metadata_with(text("litellm-principal")),
    );
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&text("litellm-principal"))
    );
}

#[test]
fn principal_extraction_skips_non_object_metadata() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), text("not-a-dict"));
    values.insert(
        "litellm_metadata".to_owned(),
        metadata_with(text("principal")),
    );
    assert_eq!(
        user_api_key_auth_from_kwargs(&values),
        Some(&text("principal"))
    );
}

#[test]
fn principal_extraction_returns_none_without_either_key() {
    let mut values = IndexMap::new();
    values.insert("metadata".to_owned(), object(&[]));
    assert_eq!(user_api_key_auth_from_kwargs(&values), None);
    assert_eq!(user_api_key_auth_from_kwargs(&IndexMap::new()), None);
}

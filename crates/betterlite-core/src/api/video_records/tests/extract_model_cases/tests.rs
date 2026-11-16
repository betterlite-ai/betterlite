#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::video_records::helpers::extract_model_from_target_model_names;

use super::fixture::assert_not_text;

#[rstest]
#[case::model_path("openai/gpt-4o", Some("openai/gpt-4o"))]
#[case::single("single", Some("single"))]
#[case::comma_separated("a,b", Some("a"))]
#[case::padded_entries("  a-model , b-model  ", Some("a-model"))]
#[case::padded_single("  foo  ", Some("foo"))]
#[case::empty("", None)]
#[case::spaces("   ", None)]
#[case::commas(",,", None)]
#[case::padded_commas(" , , ", None)]
fn extract_model_from_text_inputs(#[case] input: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        extract_model_from_target_model_names(&Json::Text(input.to_owned())).unwrap(),
        expected.map(str::to_owned),
        "{input:?}"
    );
}

#[rstest]
#[case::empty(&[][..], None)]
#[case::two_entries(&["first", "second"][..], Some("first"))]
#[case::padded_entry(&[" padded "][..], Some(" padded "))]
#[case::empty_first(&["", "foo"][..], Some(""))]
fn extract_model_from_list_inputs(#[case] names: &[&str], #[case] expected: Option<&str>) {
    let input = Json::List(
        names
            .iter()
            .map(|name| Json::Text((*name).to_owned()))
            .collect(),
    );
    assert_eq!(
        extract_model_from_target_model_names(&input).unwrap(),
        expected.map(str::to_owned),
        "{names:?}"
    );
}

#[test]
fn extract_model_from_list_with_a_null_first_entry() {
    let input = Json::List(vec![Json::Null]);
    assert_eq!(extract_model_from_target_model_names(&input).unwrap(), None);
}

#[rstest]
#[case::null(Json::Null)]
#[case::boolean_true(Json::Bool(true))]
#[case::boolean_false(Json::Bool(false))]
#[case::number(Json::Number(Number::from(12_i64)))]
#[case::object(Json::Object(IndexMap::new()))]
fn extract_model_returns_none_for_other_types(#[case] input: Json) {
    assert_eq!(
        extract_model_from_target_model_names(&input).unwrap(),
        None,
        "{input:?}"
    );
}

#[rstest]
#[case::bool(Json::Bool(true))]
#[case::number(Json::Number(Number::from(5_i64)))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn extract_model_rejects_a_non_text_list_entry(#[case] entry: Json) {
    let error = extract_model_from_target_model_names(&Json::List(vec![entry])).unwrap_err();
    assert_not_text(error, "target_model_names");
}

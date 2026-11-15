#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{map_openai_params, supported_openai_params};

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn params(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[rstest]
#[case::nomic_model("fireworks_ai/nomic-ai/nomic-embed-text-v1.5", &["dimensions"])]
#[case::nomic_anywhere("nomic-ai/anything", &["dimensions"])]
#[case::other_model("fireworks_ai/accounts/fireworks/models/bge-base", &[])]
fn supported_openai_params_depend_on_the_nomic_family(
    #[case] model: &str,
    #[case] expected: &[&str],
) {
    assert_eq!(supported_openai_params(model), expected);
}

#[test]
fn map_openai_params_copies_only_dimensions_for_a_nomic_model() {
    let non_default = params(&[
        ("dimensions", number(256)),
        ("encoding_format", text("float")),
        ("input", text("text")),
    ]);
    let mapped = map_openai_params(
        &non_default,
        IndexMap::new(),
        "fireworks_ai/nomic-ai/nomic-embed-text-v1.5",
    );
    assert_eq!(mapped, params(&[("dimensions", number(256))]));
}

#[test]
fn map_openai_params_copies_nothing_for_another_model() {
    let non_default = params(&[("dimensions", number(256))]);
    let mapped = map_openai_params(&non_default, IndexMap::new(), "fireworks_ai/bge-base");
    assert!(mapped.is_empty());
}

#[test]
fn map_openai_params_keeps_the_caller_params_and_insertion_order() {
    let non_default = params(&[
        ("dimensions", number(768)),
        ("encoding_format", text("float")),
    ]);
    let existing = params(&[("user", text("caller"))]);
    let mapped = map_openai_params(
        &non_default,
        existing,
        "fireworks_ai/nomic-ai/nomic-embed-text-v1.5",
    );
    let names: Vec<&str> = mapped.keys().map(String::as_str).collect();
    assert_eq!(names, ["user", "dimensions"]);
    assert_eq!(mapped.get("user"), Some(&text("caller")));
    assert_eq!(mapped.get("dimensions"), Some(&number(768)));
}

#[test]
fn map_openai_params_overwrites_an_existing_supported_value() {
    let non_default = params(&[("dimensions", number(768))]);
    let existing = params(&[("dimensions", number(1))]);
    let mapped = map_openai_params(
        &non_default,
        existing,
        "fireworks_ai/nomic-ai/nomic-embed-text-v1.5",
    );
    assert_eq!(mapped.get("dimensions"), Some(&number(768)));
    assert_eq!(mapped.len(), 1);
}

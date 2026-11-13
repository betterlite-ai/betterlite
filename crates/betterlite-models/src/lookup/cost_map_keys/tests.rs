#![cfg(test)]

use crate::decisions::Json;

use super::{JsonMap, backend_cost_map_keys, cost_map_backend_model};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn map(entries: &[(&str, Json)]) -> JsonMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn cost_key_prefers_model_info_base_model() {
    let params = map(&[
        ("model", text("gpt-5-deployment")),
        ("base_model", text("params-base")),
    ]);
    let info = map(&[("base_model", text("info-base"))]);
    assert_eq!(cost_map_backend_model(&params, &info), Some("info-base"));
}

#[test]
fn cost_key_empty_model_info_base_model_falls_through() {
    let params = map(&[
        ("model", text("gpt-5-deployment")),
        ("base_model", text("params-base")),
    ]);
    let info = map(&[("base_model", text(""))]);
    assert_eq!(cost_map_backend_model(&params, &info), Some("params-base"));
}

#[test]
fn cost_key_non_string_model_info_base_model_falls_through() {
    let params = map(&[
        ("model", text("gpt-5-deployment")),
        ("base_model", text("params-base")),
    ]);
    let info = map(&[("base_model", Json::Bool(true))]);
    assert_eq!(cost_map_backend_model(&params, &info), Some("params-base"));
}

#[test]
fn cost_key_litellm_params_base_model_is_second() {
    let params = map(&[
        ("model", text("gpt-5-deployment")),
        ("base_model", text("params-base")),
    ]);
    let info = map(&[]);
    assert_eq!(cost_map_backend_model(&params, &info), Some("params-base"));
}

#[test]
fn cost_key_empty_litellm_params_base_model_falls_through() {
    let params = map(&[
        ("model", text("gpt-5-deployment")),
        ("base_model", text("")),
    ]);
    let info = map(&[("base_model", Json::Null)]);
    assert_eq!(
        cost_map_backend_model(&params, &info),
        Some("gpt-5-deployment")
    );
}

#[test]
fn cost_key_model_is_the_last_resort() {
    let params = map(&[("model", text("gpt-5-deployment"))]);
    let info = map(&[]);
    assert_eq!(
        cost_map_backend_model(&params, &info),
        Some("gpt-5-deployment")
    );
}

#[test]
fn cost_key_missing_model_returns_none() {
    let params = map(&[]);
    let info = map(&[]);
    assert_eq!(cost_map_backend_model(&params, &info), None);
}

#[test]
fn cost_key_provider_prefixes_the_key() {
    assert_eq!(
        backend_cost_map_keys("gpt-5", Some("azure")),
        vec!["azure/gpt-5"]
    );
}

#[test]
fn cost_key_no_provider_keeps_the_key() {
    assert_eq!(backend_cost_map_keys("gpt-5", None), vec!["gpt-5"]);
}

#[test]
fn cost_key_empty_provider_still_prefixes() {
    assert_eq!(backend_cost_map_keys("gpt-5", Some("")), vec!["/gpt-5"]);
}

#[test]
fn cost_key_responses_twin_is_added() {
    assert_eq!(
        backend_cost_map_keys("responses/gpt-5", None),
        vec!["responses/gpt-5", "gpt-5"]
    );
}

#[test]
fn cost_key_responses_twin_with_provider() {
    assert_eq!(
        backend_cost_map_keys("responses/gpt-5", Some("openai")),
        vec!["openai/responses/gpt-5", "openai/gpt-5"]
    );
}

#[test]
fn cost_key_replaces_every_responses_segment() {
    assert_eq!(
        backend_cost_map_keys("responses/a/responses/b", None),
        vec!["responses/a/responses/b", "a/b"]
    );
}

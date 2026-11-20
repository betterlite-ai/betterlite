#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str::FromStr as _;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::{HiddenParams, Json as HiddenJson};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Number, Value, json};

use super::{
    CatalogError, EDENAI_API_BASE, EdenAiEnv, MissingApiKey, authorized_headers, cost_value,
    endpoint_url, get_error_class, insert_response_cost, parse_model_catalog, pick, reported_cost,
    reported_cost_from_bytes, reported_cost_from_headers, require_api_key, resolve_api_base,
    resolve_api_key, set_response_cost,
};
use crate::openai_base_config::{Headers, Object};

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn env(api_key: Option<&str>, api_base: Option<&str>, global: Option<&str>) -> EdenAiEnv {
    EdenAiEnv {
        api_key: api_key.map(key),
        api_base: api_base.map(str::to_owned),
        global_api_key: global.map(key),
    }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::absent(None, None, EDENAI_API_BASE)]
#[case::explicit(Some("https://proxy.example/v3"), None, "https://proxy.example/v3")]
#[case::environment(None, Some("https://gateway.example/v3"), "https://gateway.example/v3")]
#[case::explicit_wins(
    Some("https://proxy.example/v3"),
    Some("https://gateway.example/v3"),
    "https://proxy.example/v3"
)]
#[case::empty_explicit_falls_through(
    Some(""),
    Some("https://gateway.example/v3"),
    "https://gateway.example/v3"
)]
#[case::empty_environment_is_unset(None, Some(""), EDENAI_API_BASE)]
fn resolve_api_base_reads_explicit_then_environment_then_default(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let resolved = resolve_api_base(explicit, &env(None, environment, None));
    assert_eq!(resolved, expected);
}

#[test]
fn resolve_api_key_reads_the_explicit_key_then_the_environment() {
    let explicit = key("explicit");
    let environment = env(Some("environment"), None, None);
    let provided = resolve_api_key(Some(&explicit), &environment).unwrap();
    assert_eq!(provided.expose_secret(), "explicit");
    let from_environment = resolve_api_key(None, &environment).unwrap();
    assert_eq!(from_environment.expose_secret(), "environment");
    assert!(resolve_api_key(None, &env(None, None, Some("global"))).is_none());
    let empty_explicit = resolve_api_key(Some(&key("")), &environment).unwrap();
    assert_eq!(empty_explicit.expose_secret(), "environment");
}

#[rstest]
#[case::explicit(None, Some("explicit"), None, "explicit")]
#[case::global(None, None, Some("global"), "global")]
#[case::environment(None, None, None, "environment")]
#[case::explicit_beats_global(Some("explicit"), None, Some("global"), "explicit")]
#[case::global_beats_environment(Some(""), None, Some("global"), "global")]
fn require_api_key_reads_explicit_then_global_then_environment(
    #[case] explicit: Option<&str>,
    #[case] beside: Option<&str>,
    #[case] global: Option<&str>,
    #[case] expected: &str,
) {
    let environment = EdenAiEnv {
        api_key: Some(key("environment")),
        api_base: None,
        global_api_key: global.map(key),
    };
    let explicit = explicit.or(beside);
    let provided = explicit.map(key);
    let resolved = require_api_key(provided.as_ref(), "gpt-4o", &environment).unwrap();
    assert_eq!(resolved.expose_secret(), expected);
}

#[test]
fn require_api_key_without_any_key_is_the_missing_key_authentication_error() {
    let error = require_api_key(None, "gpt-4o", &env(Some(""), None, Some(""))).unwrap_err();
    assert_eq!(
        error,
        MissingApiKey {
            model: String::from("gpt-4o"),
            status_code: 401,
        }
    );
    assert_eq!(error.status_code, 401);
    assert!(matches!(error.class(), ErrorClass::Authentication(_)));
    assert_eq!(
        error.to_string(),
        "Missing Eden AI API key: set EDENAI_API_KEY or pass api_key"
    );
}

#[test]
fn authorized_headers_adds_a_bearer_and_replaces_the_callers_authorization() {
    let supplied = headers(&[("Authorization", "Bearer stale"), ("x-trace", "1")]);
    let merged =
        authorized_headers(&supplied, Some(&key("eden")), "m", &env(None, None, None)).unwrap();
    assert_eq!(
        merged,
        headers(&[("Authorization", "Bearer eden"), ("x-trace", "1")])
    );
}

#[test]
fn authorized_headers_replaces_an_authorization_spelled_in_another_case() {
    let supplied = headers(&[("authorization", "Bearer stale")]);
    let merged =
        authorized_headers(&supplied, Some(&key("eden")), "m", &env(None, None, None)).unwrap();
    assert_eq!(merged, headers(&[("Authorization", "Bearer eden")]));
}

#[test]
fn authorized_headers_without_a_key_is_the_missing_key_error() {
    let error = authorized_headers(&Headers::new(), None, "m", &env(None, None, None)).unwrap_err();
    assert_eq!(error.status_code, 401);
}

#[rstest]
#[case::default_base(None, None, "https://api.edenai.run/v3/models")]
#[case::explicit(
    Some("https://proxy.example/v3"),
    None,
    "https://proxy.example/v3/models"
)]
#[case::trailing_slashes(
    Some("https://proxy.example/v3///"),
    None,
    "https://proxy.example/v3/models"
)]
#[case::environment(
    None,
    Some("https://gateway.example/v3"),
    "https://gateway.example/v3/models"
)]
fn endpoint_url_trims_trailing_slashes_then_appends_the_path(
    #[case] api_base: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let url = endpoint_url(api_base, "models", &env(None, environment, None));
    assert_eq!(url, expected);
}

#[test]
fn pick_keeps_only_the_listed_keys_from_the_params() {
    let params = Object::from_iter([
        (String::from("user"), json!("alice")),
        (String::from("dimensions"), json!(8)),
        (String::from("model"), json!("m")),
    ]);
    let picked = pick(&params, &["dimensions", "user"]);
    assert_eq!(
        picked,
        Object::from_iter([
            (String::from("dimensions"), json!(8)),
            (String::from("user"), json!("alice")),
        ])
    );
    assert!(pick(&params, &[]).is_empty());
}

#[rstest]
#[case::integer(json!({"cost": 2}), Some("2"))]
#[case::fraction(json!({"cost": 2.25}), Some("2.25"))]
#[case::numeric_string(json!({"cost": "2.25"}), Some("2.25"))]
#[case::padded_string(json!({"cost": " 0.10 "}), Some("0.10"))]
#[case::scientific_string(json!({"cost": "1e3"}), Some("1000"))]
#[case::sign_and_leading_dot(json!({"cost": "+.5"}), Some("0.5"))]
#[case::trailing_dot(json!({"cost": "1."}), Some("1"))]
#[case::negative(json!({"cost": "-2"}), Some("-2"))]
#[case::boolean_true(json!({"cost": true}), Some("1"))]
#[case::boolean_false(json!({"cost": false}), Some("0"))]
#[case::null(json!({"cost": null}), None)]
#[case::absent(json!({"other": 1}), None)]
#[case::header_alias(json!({"x-edenai-cost": 3}), Some("3"))]
#[case::cost_wins(json!({"cost": 1, "x-edenai-cost": 2}), Some("1"))]
#[case::null_cost_wins(json!({"cost": null, "x-edenai-cost": 2}), None)]
#[case::invalid_cost_wins(json!({"cost": "abc", "x-edenai-cost": 2}), None)]
#[case::invalid_header_alias(json!({"x-edenai-cost": {}}), None)]
#[case::empty_string(json!({"cost": ""}), None)]
#[case::blank_string(json!({"cost": "   "}), None)]
#[case::underscore(json!({"cost": "1_0"}), None)]
#[case::infinity(json!({"cost": "inf"}), None)]
#[case::out_of_range(json!({"cost": "1e400"}), None)]
#[case::list(json!({"cost": [1]}), None)]
#[case::object_value(json!({"cost": {"amount": 1}}), None)]
#[case::not_an_object(json!(5), None)]
#[case::array_body(json!([{"cost": 1}]), None)]
#[case::null_body(json!(null), None)]
fn reported_cost_reads_the_cost_alias_from_a_json_value(
    #[case] payload: Value,
    #[case] expected: Option<&str>,
) {
    let found = reported_cost(&payload);
    assert_eq!(
        found,
        expected.map(|text| Decimal::from_str_exact(text).unwrap())
    );
}

#[test]
fn cost_value_reads_a_bare_cost() {
    assert_eq!(cost_value(&json!("1.5")), Some(Decimal::new(15, 1)));
    assert_eq!(cost_value(&json!(null)), None);
}

#[rstest]
#[case::object(br#"{"cost": 1.5}"#, Some(Decimal::new(15, 1)))]
#[case::header_alias(br#"{"x-edenai-cost": "4"}"#, Some(Decimal::from(4)))]
#[case::not_json(b"not json", None)]
#[case::json_text(br#""text""#, None)]
#[case::json_null(b"null", None)]
#[case::json_array(b"[]", None)]
#[case::invalid_value(br#"{"cost": "abc"}"#, None)]
fn reported_cost_from_bytes_reads_only_a_json_object(
    #[case] body: &[u8],
    #[case] expected: Option<Decimal>,
) {
    assert_eq!(reported_cost_from_bytes(body), expected);
}

#[rstest]
#[case::header(&[("x-edenai-cost", "2.5")], Some(2.5))]
#[case::upper_case(&[("X-EdenAI-Cost", "2.5")], Some(2.5))]
#[case::plain_cost(&[("cost", "3")], Some(3.0))]
#[case::cost_wins(&[("x-edenai-cost", "2"), ("cost", "1")], Some(1.0))]
#[case::invalid(&[("x-edenai-cost", "abc")], None)]
#[case::absent(&[("x-request-id", "1")], None)]
fn reported_cost_from_headers_tries_both_aliases_in_any_case(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Option<f64>,
) {
    let found = reported_cost_from_headers(&headers(pairs));
    assert_eq!(
        found,
        expected.map(|value| Decimal::from_str_exact(&value.to_string()).unwrap())
    );
}

#[test]
fn set_response_cost_leaves_hidden_params_alone_without_a_cost() {
    let mut hidden = HiddenParams::new();
    set_response_cost(&mut hidden, None);
    assert!(hidden.is_empty());
}

#[test]
fn set_response_cost_builds_the_additional_headers_object() {
    let mut hidden = HiddenParams::new();
    set_response_cost(&mut hidden, Some(Decimal::new(25, 1)));
    let additional = HiddenJson::Object(IndexMap::from([(
        String::from("llm_provider-x-litellm-response-cost"),
        HiddenJson::Number(Number::from_str("0.25").unwrap()),
    )]));
    assert_eq!(
        hidden.get("additional_headers"),
        Some(&additional),
        "{hidden:?}"
    );
}

#[test]
fn set_response_cost_keeps_existing_additional_headers() {
    let mut hidden = HiddenParams::new();
    hidden.insert(
        String::from("additional_headers"),
        HiddenJson::Object(IndexMap::from([(
            String::from("llm_provider-x-request-id"),
            HiddenJson::Text(String::from("req-1")),
        )])),
    );
    set_response_cost(&mut hidden, Some(Decimal::from(1)));
    let Some(HiddenJson::Object(additional)) = hidden.get("additional_headers") else {
        panic!("the additional headers stay an object");
    };
    assert_eq!(
        additional.get("llm_provider-x-request-id"),
        Some(&HiddenJson::Text(String::from("req-1")))
    );
    assert_eq!(
        additional.get("llm_provider-x-litellm-response-cost"),
        Some(&HiddenJson::Number(Number::from_str("1").unwrap()))
    );
}

#[test]
fn set_response_cost_replaces_additional_headers_that_are_not_an_object() {
    let mut hidden = HiddenParams::new();
    hidden.insert(
        String::from("additional_headers"),
        HiddenJson::Text(String::from("broken")),
    );
    set_response_cost(&mut hidden, Some(Decimal::from(2)));
    let Some(HiddenJson::Object(additional)) = hidden.get("additional_headers") else {
        panic!("a non-object becomes the cost object");
    };
    assert_eq!(
        additional.get("llm_provider-x-litellm-response-cost"),
        Some(&HiddenJson::Number(Number::from_str("2").unwrap()))
    );
}

#[test]
fn insert_response_cost_writes_into_the_header_map() {
    let mut headers = IndexMap::new();
    insert_response_cost(&mut headers, None);
    assert!(headers.is_empty());
    insert_response_cost(&mut headers, Some(Decimal::new(5, 1)));
    assert_eq!(
        headers.get("llm_provider-x-litellm-response-cost"),
        Some(&HiddenJson::Number(Number::from_str("0.5").unwrap()))
    );
}

#[test]
fn get_error_class_builds_the_eden_exception_from_the_reply() {
    let supplied = headers(&[("x-request-id", "req-1")]);
    let error = get_error_class("boom", 429, Some(&supplied));
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "boom");
    assert_eq!(error.headers, supplied);
    assert_eq!(error.to_string(), "boom");
    let without_headers = get_error_class("down", 500, None);
    assert!(without_headers.headers.is_empty());
}

#[rstest]
#[case::empty(br#"{"data":[{"id":"a"},{"id":"b"}]}"#, &["a", "b"])]
#[case::no_entries(br#"{"data":[]}"#, &[])]
#[case::extra_fields(br#"{"data":[{"id":"a","object":"model"}],"object":"list"}"#, &["a"])]
fn parse_model_catalog_reads_the_ids_in_order(#[case] body: &[u8], #[case] expected: &[&str]) {
    let ids = parse_model_catalog(body).unwrap();
    assert_eq!(
        ids,
        expected
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>()
    );
}

#[rstest]
#[case::not_json(b"not json", CatalogError::Json)]
#[case::json_null(b"null", CatalogError::Data)]
#[case::json_array(b"[]", CatalogError::Data)]
#[case::no_data(b"{}", CatalogError::Data)]
#[case::data_not_a_list(br#"{"data":{}}"#, CatalogError::Data)]
#[case::entry_not_an_object(br#"{"data":[1]}"#, CatalogError::Id)]
#[case::no_id(br#"{"data":[{}]}"#, CatalogError::Id)]
#[case::id_not_a_string(br#"{"data":[{"id":1}]}"#, CatalogError::Id)]
fn parse_model_catalog_names_the_missing_field(
    #[case] body: &[u8],
    #[case] expected: CatalogError,
) {
    assert_eq!(parse_model_catalog(body), Err(expected));
    assert!(!expected.to_string().is_empty());
}

#[rstest]
fn replays_the_edenai_cost_reply_corpus(#[files("fuzz/corpus/edenai_cost_reply/*")] path: PathBuf) {
    let body = fs::read(path).expect("the corpus fixture");
    let _cost = reported_cost_from_bytes(&body);
    let _catalog = parse_model_catalog(&body);
}

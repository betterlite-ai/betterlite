use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::SearchQuery;
use serde_json::json;

use crate::search::AzureSearchConfig;

use super::support::{
    citation_body, connection_settings, json_value, model_settings, params, response_with_params,
};

pub(super) fn max_results_invalid_default() {
    web_ignores_invalid_max_results();
    connection_omits_invalid_count();
    reply_without_params_is_uncapped();
}

fn web_ignores_invalid_max_results() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let body = citation_body(&[
        "https://a.example",
        "https://b.example",
        "https://c.example",
        "https://d.example",
    ]);
    assert_web_request_and_results(&config, &params("{}"), &body);
    assert_web_request_and_results(&config, &params(r#"{"max_results":0}"#), &body);
    assert_web_request_and_results(&config, &params(r#"{"max_results":-1}"#), &body);
    assert_web_request_and_results(&config, &params(r#"{"max_results":true}"#), &body);
    assert_web_request_and_results(&config, &params(r#"{"max_results":"5"}"#), &body);
    assert_web_request_and_results(&config, &params(r#"{"max_results":2.5}"#), &body);
    assert_web_request_and_results(&config, &params(r#"{"max_results":null}"#), &body);
    assert_web_request_and_results(
        &config,
        &params(r#"{"max_results":9223372036854775808}"#),
        &body,
    );
}

fn assert_web_request_and_results(config: &AzureSearchConfig<'_>, optional: &Object, body: &str) {
    let request = config
        .transform_search_request(&SearchQuery::Single(String::from("query")), optional)
        .unwrap();
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["tools"], json_value(r#"[{"type":"web_search"}]"#));
    let response = response_with_params(config, body, Some(optional)).unwrap();
    assert_eq!(response.results.len(), 4_usize);
}

fn connection_omits_invalid_count() {
    let settings = connection_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    assert_count_is_absent(&config, &params("{}"));
    assert_count_is_absent(&config, &params(r#"{"max_results":0}"#));
    assert_count_is_absent(&config, &params(r#"{"max_results":-1}"#));
    assert_count_is_absent(&config, &params(r#"{"max_results":true}"#));
    assert_count_is_absent(&config, &params(r#"{"max_results":"5"}"#));
    assert_count_is_absent(&config, &params(r#"{"max_results":2.5}"#));
    assert_count_is_absent(&config, &params(r#"{"max_results":null}"#));
    assert_count_is_absent(&config, &params(r#"{"max_results":9223372036854775808}"#));
}

fn assert_count_is_absent(config: &AzureSearchConfig<'_>, optional: &Object) {
    let request = config
        .transform_search_request(&SearchQuery::Single(String::from("query")), optional)
        .unwrap();
    let body = serde_json::to_value(&request).unwrap();
    let configuration = &body["tools"][0]["bing_grounding"]["search_configurations"][0];
    assert_eq!(
        configuration["project_connection_id"],
        json!("connection-1")
    );
    assert_eq!(configuration.get("count"), None);
}

fn reply_without_params_is_uncapped() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let body = citation_body(&["https://a.example", "https://b.example"]);
    let response = response_with_params(&config, &body, None).unwrap();
    assert_eq!(response.results.len(), 2_usize);
}

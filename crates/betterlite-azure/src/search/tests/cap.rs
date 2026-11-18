use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::SearchQuery;
use serde_json::json;

use crate::search::AzureSearchConfig;

use super::support::{
    citation_body, connection_settings, model_settings, object, response_with_params,
};

const URLS: [&str; 3] = [
    "https://a.example",
    "https://b.example",
    "https://c.example",
];

pub(super) fn max_results_cap() {
    web_mode_caps_the_results();
    connection_mode_caps_and_hints();
    invalid_values_leave_the_results_alone();
}

fn web_mode_caps_the_results() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    assert_capped(&config, json!({"max_results": 2}), 2_usize);
    assert_capped(&config, json!({"max_results": 1}), 1_usize);
    assert_capped(&config, json!({"max_results": 3}), 3_usize);
    assert_capped(&config, json!({"max_results": 10}), 3_usize);
}

fn connection_mode_caps_and_hints() {
    let settings = connection_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    assert_capped(&config, json!({"max_results": 2}), 2_usize);
    let params = object(json!({"max_results": 2}));
    let request = config
        .transform_search_request(&SearchQuery::Single(String::from("query")), &params)
        .unwrap();
    let body = serde_json::to_value(&request).unwrap();
    assert_eq!(
        body["tools"][0]["bing_grounding"]["search_configurations"][0]["count"],
        json!(2)
    );
}

fn invalid_values_leave_the_results_alone() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    assert_capped(&config, json!({"max_results": 0}), 3_usize);
    assert_capped(&config, json!({"max_results": null}), 3_usize);
}

fn assert_capped(config: &AzureSearchConfig<'_>, value: JsonValue, expected: usize) {
    let params = object(value);
    let body = citation_body(&URLS);
    let response = response_with_params(config, &body, Some(&params)).unwrap();
    let urls: Vec<&str> = response
        .results
        .iter()
        .map(|result| result.url.as_str())
        .collect();
    assert_eq!(urls, URLS[..expected].to_vec());
}

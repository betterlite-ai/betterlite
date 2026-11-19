use betterlite_config::sections::azure_search::Settings;
use betterlite_openai::search::records::SearchResponse;
use serde_json::json;

use crate::search::{AzureSearchConfig, RESPONSE_COST_HEADER};

use super::support::{
    MODEL, citation_body, connection_settings, model_settings, params, response_with_params,
};

pub(super) fn search_response_price() {
    web_mode_zeroes_the_cost();
    connection_mode_leaves_pricing_alone();
    blank_connection_id_is_web_mode();
    capped_results_keep_the_metadata();
}

fn priced_response(settings: &Settings) -> SearchResponse {
    let config = AzureSearchConfig {
        settings,
        minter: None,
    };
    let body = citation_body(&["https://a.example", "https://b.example"]);
    response_with_params(&config, &body, None).unwrap()
}

fn web_mode_zeroes_the_cost() {
    let settings = model_settings();
    let response = priced_response(&settings);
    assert_eq!(response.object.0, "search");
    assert_eq!(response.results.len(), 2_usize);
    assert!(response.extra.is_empty());
    let additional = response.hidden_params.get("additional_headers").unwrap();
    assert_eq!(additional[RESPONSE_COST_HEADER], json!(0.0_f64));
    assert!(additional[RESPONSE_COST_HEADER].is_f64());
    assert_eq!(response.hidden_params.len(), 1_usize);
}

fn connection_mode_leaves_pricing_alone() {
    let settings = connection_settings();
    let response = priced_response(&settings);
    assert_eq!(response.object.0, "search");
    assert!(response.hidden_params.is_empty());
}

fn blank_connection_id_is_web_mode() {
    let settings = Settings {
        project_endpoint: None,
        model: Some(String::from(MODEL)),
        connection_id: Some(String::new()),
        token: None,
    };
    let response = priced_response(&settings);
    assert!(!response.hidden_params.is_empty());
}

fn capped_results_keep_the_metadata() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let body = citation_body(&["https://a.example", "https://b.example"]);
    let optional = params(r#"{"max_results": 1}"#);
    let response = response_with_params(&config, &body, Some(&optional)).unwrap();
    assert_eq!(response.results.len(), 1_usize);
    assert_eq!(response.results[0].url, "https://a.example");
    assert_eq!(response.object.0, "search");
    assert!(!response.hidden_params.is_empty());
}

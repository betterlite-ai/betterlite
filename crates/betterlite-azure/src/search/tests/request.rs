use betterlite_config::sections::azure_search::Settings;
use betterlite_openai::search::provider::SearchQuery;
use serde_json::json;

use crate::search::AzureSearchConfig;

use super::support::{
    MODEL, connection_settings, json_value, model_settings, params, request_body,
};

pub(super) fn search_request_web_tool() {
    web_tool_body();
    web_tool_country();
    connection_tool_body();
    connection_tool_ignores_country();
    blank_connection_id_is_web_mode();
    joined_query();
}

fn web_tool_body() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let optional = params(
        r#"{"max_results":5,"search_domain_filter":["example.com"],"max_tokens_per_page":10}"#,
    );
    let body = request_body(&config, &optional);
    let expected = format!(
        r#"{{"model":"{MODEL}","input":"azure grounding query","tools":[{{"type":"web_search"}}]}}"#
    );
    assert_eq!(body, json_value(&expected));
}

fn web_tool_country() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let located = request_body(&config, &params(r#"{"country": "de"}"#));
    assert_eq!(
        located["tools"][0]["user_location"],
        json_value(r#"{"type":"approximate","country":"DE"}"#)
    );
    let empty = request_body(&config, &params(r#"{"country": ""}"#));
    assert_eq!(
        empty["tools"][0]["user_location"],
        json_value(r#"{"type":"approximate","country":""}"#)
    );
    let numeric = request_body(&config, &params(r#"{"country": 42}"#));
    assert!(numeric["tools"][0].get("user_location").is_none());
    let absent = request_body(&config, &params("{}"));
    assert!(absent["tools"][0].get("user_location").is_none());
}

fn connection_tool_body() {
    let settings = connection_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let body = request_body(&config, &params(r#"{"max_results": 5}"#));
    assert_eq!(body["model"], json!(MODEL));
    assert_eq!(body["input"], json!("azure grounding query"));
    let tool = &body["tools"][0];
    assert_eq!(tool["type"], json!("bing_grounding"));
    assert_eq!(
        tool["bing_grounding"]["search_configurations"],
        json_value(r#"[{"project_connection_id":"connection-1","count":5}]"#)
    );
}

fn connection_tool_ignores_country() {
    let settings = connection_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let optional = params(
        r#"{"country":"de","max_results":2,"search_domain_filter":["example.com"],"max_tokens_per_page":4}"#,
    );
    let body = request_body(&config, &optional);
    let tool = &body["tools"][0];
    assert_eq!(tool["type"], json!("bing_grounding"));
    assert_eq!(
        tool["bing_grounding"]["search_configurations"],
        json_value(r#"[{"project_connection_id":"connection-1","count":2}]"#)
    );
    assert!(tool.get("user_location").is_none());
}

fn blank_connection_id_is_web_mode() {
    let settings = Settings {
        connection_id: Some(String::new()),
        ..model_settings()
    };
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let body = request_body(&config, &params(r#"{"max_results": 2}"#));
    assert_eq!(body["tools"], json_value(r#"[{"type":"web_search"}]"#));
}

fn joined_query() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let optional = params("{}");
    let joined = config
        .transform_search_request(
            &SearchQuery::Multiple(vec![String::from("first"), String::from("second")]),
            &optional,
        )
        .unwrap();
    assert_eq!(joined.input, "first second");
    let empty = config
        .transform_search_request(&SearchQuery::Multiple(Vec::new()), &optional)
        .unwrap();
    assert_eq!(empty.input, "");
}

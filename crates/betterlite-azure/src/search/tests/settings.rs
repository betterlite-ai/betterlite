use betterlite_config::sections::azure_search::{MODEL_ENV, PROJECT_ENDPOINT_ENV, Settings};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::SearchQuery;
use serde_json::json;

use crate::search::error::AzureSearchError;
use crate::search::{AzureSearchConfig, RESPONSES_PATH};

use super::support::{MODEL, endpoint_settings, model_settings, object, request_body};

const ENDPOINT: &str = "https://project.services.ai.azure.com/api/projects/example";

pub(super) fn search_settings() {
    complete_url_appends_the_responses_path();
    complete_url_requires_an_endpoint();
    request_requires_a_model();
    configured_model_reaches_the_request();
    query_lists_join_with_one_space();
}

fn complete_url_appends_the_responses_path() {
    let settings = endpoint_settings(&format!("{ENDPOINT}/"));
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let expected = format!("{ENDPOINT}{RESPONSES_PATH}");
    assert_eq!(config.complete_url(None).unwrap(), expected);
    assert_eq!(config.complete_url(Some("")).unwrap(), expected);
    assert_eq!(
        config
            .complete_url(Some("https://caller.example/base///"))
            .unwrap(),
        "https://caller.example/base/openai/v1/responses"
    );
    assert_eq!(config.complete_url(Some(&expected)).unwrap(), expected);
    assert_eq!(
        config.complete_url(Some(&format!("{expected}/"))).unwrap(),
        expected
    );
}

fn complete_url_requires_an_endpoint() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let error = config.complete_url(None).unwrap_err();
    let message = error.to_string();
    assert!(message.contains(PROJECT_ENDPOINT_ENV));
    assert!(message.contains("https://<account>.services.ai.azure.com/api/projects/<project>"));
    assert!(matches!(error, AzureSearchError::MissingProjectEndpoint));
    let blank = Settings {
        project_endpoint: Some(String::new()),
        model: Some(String::from(MODEL)),
        connection_id: None,
        token: None,
    };
    let blank_config = AzureSearchConfig {
        settings: &blank,
        minter: None,
    };
    assert!(matches!(
        blank_config.complete_url(None).unwrap_err(),
        AzureSearchError::MissingProjectEndpoint
    ));
    assert!(matches!(
        config.complete_url(Some("")).unwrap_err(),
        AzureSearchError::MissingProjectEndpoint
    ));
}

fn request_requires_a_model() {
    let absent = Settings {
        project_endpoint: Some(String::from(ENDPOINT)),
        model: None,
        connection_id: None,
        token: None,
    };
    let config = AzureSearchConfig {
        settings: &absent,
        minter: None,
    };
    let error = config
        .transform_search_request(&SearchQuery::Single(String::from("query")), &Object::new())
        .unwrap_err();
    assert!(matches!(error, AzureSearchError::MissingModel));
    assert!(error.to_string().contains(MODEL_ENV));
    assert!(error.to_string().contains("gpt-4.1"));
    let blank = Settings {
        project_endpoint: Some(String::from(ENDPOINT)),
        model: Some(String::new()),
        connection_id: None,
        token: None,
    };
    let blank_config = AzureSearchConfig {
        settings: &blank,
        minter: None,
    };
    assert!(matches!(
        blank_config
            .transform_search_request(&SearchQuery::Single(String::from("query")), &Object::new())
            .unwrap_err(),
        AzureSearchError::MissingModel
    ));
}

fn configured_model_reaches_the_request() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    assert_eq!(request_body(&config, &Object::new())["model"], json!(MODEL));
}

fn query_lists_join_with_one_space() {
    let settings = model_settings();
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let params = object(json!({}));
    let joined = config
        .transform_search_request(
            &SearchQuery::Multiple(vec![String::from("first"), String::from("second")]),
            &params,
        )
        .unwrap();
    assert_eq!(joined.input, "first second");
    let empty = config
        .transform_search_request(&SearchQuery::Multiple(Vec::new()), &params)
        .unwrap();
    assert_eq!(empty.input, "");
}

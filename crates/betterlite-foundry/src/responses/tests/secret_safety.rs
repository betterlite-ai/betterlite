use std::fmt::{Debug, Display};

use betterlite_prompts::arguments::json::Json;

use crate::credentials::model_info::ModelInfoAccessors;
use crate::responses::auth::{EnvironmentRequest, validate_environment};
use crate::responses::tests::support::{FailingSources, environment, headers, object};
use crate::responses::url::complete_url;

const SENTINEL: &str = "sentinel-secret-value";

pub(super) fn secret_safe_failure() {
    missing_credentials_hides_secret();
    provider_failure_hides_secret();
    invalid_base_hides_secret();
}

fn assert_hidden(error: &(impl Display + Debug)) {
    let displayed = error.to_string();
    let debugged = format!("{error:?}");
    assert!(!displayed.contains(SENTINEL), "{displayed}");
    assert!(!debugged.contains(SENTINEL), "{debugged}");
}

fn missing_credentials_hides_secret() {
    let caller = headers(&[]);
    let env = environment(&[("AZURE_CLIENT_SECRET", SENTINEL)]);
    let sources = FailingSources;
    let error = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: ModelInfoAccessors {
            global_api_base: None,
            global_api_key: None,
            environment_api_base: None,
            environment_api_key: None,
        },
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err();
    assert!(error.to_string().contains("AZURE_AI_API_KEY"));
    assert_hidden(&error);
}

fn provider_failure_hides_secret() {
    let caller = headers(&[]);
    let env = environment(&[]);
    let sources = FailingSources;
    let params = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text(SENTINEL.to_owned())),
    ]);
    let error = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: ModelInfoAccessors {
            global_api_base: None,
            global_api_key: None,
            environment_api_base: None,
            environment_api_key: None,
        },
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params: Some(&params),
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err();
    assert!(error.to_string().contains("provider refused"));
    assert_hidden(&error);
}

fn invalid_base_hides_secret() {
    let sources = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    };
    let bad_port = complete_url(
        sources,
        Some("https://user:sentinel-secret-value@res.services.ai.azure.com:port"),
    )
    .unwrap_err();
    assert!(bad_port.to_string().contains("Invalid port"));
    assert_hidden(&bad_port);
    let unrewritable = complete_url(
        sources,
        Some("https://user:sentinel-secret-value@res.services.ai.azure.com/api/projects/pro%3Fj"),
    )
    .unwrap_err();
    assert_hidden(&unrewritable);
}

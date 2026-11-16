use std::collections::BTreeMap;
use std::fmt::{Debug, Display};

use betterlite_prompts::arguments::json::{Json, Object};

use crate::credentials::error::CredentialsError;
use crate::responses::auth::{EnvironmentRequest, validate_environment};
use crate::responses::tests::support::{
    FailingSources, environment, headers, no_accessors, object,
};
use crate::responses::url::complete_url;

const SENTINEL: &str = "sentinel-secret-value";

pub(super) fn secret_safe_failure() {
    missing_credentials_hides_secret();
    provider_failure_hides_secret();
    invalid_base_hides_secret();
}

fn assert_hidden<Failure>(error: &Failure)
where
    Failure: Display + Debug,
{
    let displayed = error.to_string();
    let debugged = format!("{error:?}");
    assert!(!displayed.contains(SENTINEL), "{displayed}");
    assert!(!debugged.contains(SENTINEL), "{debugged}");
}

fn auth_error(environment: &BTreeMap<String, String>, params: Option<&Object>) -> CredentialsError {
    let caller = headers(&[]);
    let sources = FailingSources;
    validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: no_accessors(),
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params,
        environment,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err()
}

fn missing_credentials_hides_secret() {
    let env = environment(&[("AZURE_CLIENT_SECRET", SENTINEL)]);
    let error = auth_error(&env, None);
    assert!(error.to_string().contains("AZURE_AI_API_KEY"));
    assert_hidden(&error);
}

fn provider_failure_hides_secret() {
    let env = environment(&[]);
    let params = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text(SENTINEL.to_owned())),
    ]);
    let error = auth_error(&env, Some(&params));
    assert!(error.to_string().contains("provider refused"));
    assert_hidden(&error);
}

fn invalid_base_hides_secret() {
    let bad_port = complete_url(
        no_accessors(),
        Some("https://user:sentinel-secret-value@res.services.ai.azure.com:port"),
    )
    .unwrap_err();
    assert!(bad_port.to_string().contains("Invalid port"));
    assert_hidden(&bad_port);
    let unrewritable = complete_url(
        no_accessors(),
        Some("https://user:sentinel-secret-value@res.services.ai.azure.com/api/projects/pro%3Fj"),
    )
    .unwrap_err();
    assert_hidden(&unrewritable);
}

use betterlite_foundry::vector_stores::environment::{
    EnvironmentRequest, Headers, Params, auth_credentials, validate_environment,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::super::fixtures::{NoSources, empty_credentials};
use super::super::scenarios::{AuthCredentialsInput, HeaderScenario};

pub(super) fn credentials_report(input: &AuthCredentialsInput) -> Value {
    let credentials: Vec<Value> = input
        .credentials
        .iter()
        .map(credential_outcome)
        .collect();
    let environments: Vec<Value> = input
        .environments
        .iter()
        .map(header_report)
        .collect();
    json!({"credentials": credentials, "environments": environments})
}

fn credential_outcome(params: &Params) -> Value {
    match auth_credentials(params) {
        Ok(credentials) => json!({"ok": true, "headers": exposed(&credentials.headers)}),
        Err(_error) => json!({"ok": false}),
    }
}

fn header_report(scenario: &HeaderScenario) -> Value {
    let credentials = empty_credentials();
    let sources = NoSources;
    let key = scenario
        .api_key
        .as_ref()
        .map(|key| SecretString::from(key.as_str()));
    let headers: Headers = scenario
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    let request = EnvironmentRequest {
        headers,
        api_key: key.as_ref(),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    };
    match validate_environment(request) {
        Ok(headers) => json!({"ok": true, "headers": exposed(&headers)}),
        Err(_error) => json!({"ok": false}),
    }
}

fn exposed(headers: &Headers) -> Value {
    let fields: serde_json::Map<String, Value> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                Value::String(value.expose_secret().to_owned()),
            )
        })
        .collect();
    Value::Object(fields)
}

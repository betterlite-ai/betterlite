use betterlite_azure::responses::request::transform_request;
use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::credentials::error::CredentialsError;
use crate::credentials::model_info::ModelInfoAccessors;
use crate::responses::auth::{EnvironmentRequest, validate_environment};
use crate::responses::tests::support::{
    FixtureSources, environment, headers, headers_for, no_accessors, object,
};

pub(in crate::responses::tests) fn responses_auth() {
    explicit_key_headers();
    host_specific_header();
    entra_fallback();
    key_resolution();
    header_follows_base();
    missing_credentials();
    empty_key_falls_back_to_entra();
    inherited_request_filtering();
}

fn explicit_key_headers() {
    let caller = headers(&[("X-Caller", "1"), ("Authorization", "Bearer stale")]);
    let key = SecretString::from("case-key".to_owned());
    let validated = headers_for(
        &caller,
        no_accessors(),
        Some("https://res.services.ai.azure.com"),
        Some(&key),
    );
    assert_eq!(
        validated.get("api-key").map(String::as_str),
        Some("case-key")
    );
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer stale")
    );
    assert_eq!(validated.get("X-Caller").map(String::as_str), Some("1"));
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

fn host_specific_header() {
    let caller = headers(&[]);
    let key = SecretString::from("case-key".to_owned());
    let api_key_host = headers_for(
        &caller,
        no_accessors(),
        Some("https://res.openai.azure.com"),
        Some(&key),
    );
    assert_eq!(
        api_key_host.get("api-key").map(String::as_str),
        Some("case-key")
    );
    assert_eq!(api_key_host.get("Authorization"), None);
    let bearer_host = headers_for(
        &caller,
        no_accessors(),
        Some("https://api.example.com"),
        Some(&key),
    );
    assert_eq!(
        bearer_host.get("Authorization").map(String::as_str),
        Some("Bearer case-key")
    );
    assert_eq!(bearer_host.get("api-key"), None);
}

fn entra_fallback() {
    let caller = headers(&[]);
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let params = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("secret".to_owned())),
    ]);
    let validated = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: no_accessors(),
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params: Some(&params),
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    let expected = format!(
        "Bearer client_secret tenant=tenant client=client secret=secret scope={DEFAULT_SCOPE}"
    );
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some(expected.as_str())
    );
}

fn key_resolution() {
    environment_key();
    layered_key();
}

fn environment_key() {
    let caller = headers(&[]);
    let env_key = SecretString::from("env-key".to_owned());
    let accessors = ModelInfoAccessors {
        environment_api_key: Some(&env_key),
        ..no_accessors()
    };
    let resolved = headers_for(
        &caller,
        accessors,
        Some("https://res.services.ai.azure.com"),
        None,
    );
    assert_eq!(resolved.get("api-key").map(String::as_str), Some("env-key"));
}

fn layered_key() {
    let caller = headers(&[]);
    let env_key = SecretString::from("env-key".to_owned());
    let global_key = SecretString::from("global-key".to_owned());
    let explicit_key = SecretString::from("explicit-key".to_owned());
    let layered = ModelInfoAccessors {
        global_api_key: Some(&global_key),
        environment_api_key: Some(&env_key),
        ..no_accessors()
    };
    let from_global = headers_for(
        &caller,
        layered,
        Some("https://res.services.ai.azure.com"),
        None,
    );
    assert_eq!(
        from_global.get("api-key").map(String::as_str),
        Some("global-key")
    );
    let from_explicit = headers_for(
        &caller,
        layered,
        Some("https://res.services.ai.azure.com"),
        Some(&explicit_key),
    );
    assert_eq!(
        from_explicit.get("api-key").map(String::as_str),
        Some("explicit-key")
    );
}

fn header_follows_base() {
    let caller = headers(&[]);
    let key = SecretString::from("case-key".to_owned());
    let global = ModelInfoAccessors {
        global_api_base: Some("https://res.services.ai.azure.com"),
        ..no_accessors()
    };
    let from_global = headers_for(&caller, global, None, Some(&key));
    assert_eq!(
        from_global.get("api-key").map(String::as_str),
        Some("case-key")
    );
    let foreign = ModelInfoAccessors {
        global_api_base: Some("https://api.example.com"),
        ..no_accessors()
    };
    let from_explicit = headers_for(
        &caller,
        foreign,
        Some("https://res.services.ai.azure.com"),
        Some(&key),
    );
    assert_eq!(
        from_explicit.get("api-key").map(String::as_str),
        Some("case-key")
    );
}

fn missing_credentials() {
    let caller = headers(&[]);
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let error = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: no_accessors(),
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err();
    assert!(matches!(error, CredentialsError::MissingCredentials { .. }));
    assert!(error.to_string().contains("AZURE_AI_API_KEY"));
}

// an empty AZURE_AI_API_KEY is not a key to LiteLLM's auth headers, so the Entra credentials answer instead
fn empty_key_falls_back_to_entra() {
    let caller = headers(&[]);
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let empty = SecretString::from(String::new());
    let accessors = ModelInfoAccessors {
        environment_api_key: Some(&empty),
        ..no_accessors()
    };
    let params = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("secret".to_owned())),
    ]);
    let validated = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors,
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params: Some(&params),
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    let expected = format!(
        "Bearer client_secret tenant=tenant client=client secret=secret scope={DEFAULT_SCOPE}"
    );
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some(expected.as_str())
    );
    assert_eq!(validated.get("api-key"), None);
}

// the Foundry endpoint leaves the request transform to B544, which filters reasoning and message status
fn inherited_request_filtering() {
    let input = ResponsesInput::Items(vec![
        ResponsesItem(Json::Object(object([
            ("type", Json::Text("reasoning".to_owned())),
            ("id", Json::Text("rs_1".to_owned())),
            ("summary", Json::List(Vec::new())),
            ("status", Json::Text("completed".to_owned())),
        ]))),
        ResponsesItem(Json::Object(object([
            ("type", Json::Text("message".to_owned())),
            ("role", Json::Text("user".to_owned())),
            ("content", Json::Text("hi".to_owned())),
            ("status", Json::Text("completed".to_owned())),
        ]))),
    ]);
    let body = transform_request("azure_ai/gpt", &input, &Object::new(), None).unwrap();
    let Some(Json::List(items)) = body.get("input") else {
        panic!("the transform writes the item list")
    };
    assert_eq!(items.len(), 2);
    assert_status_dropped(items);
    let Some(Json::Object(reasoning)) = items.first() else {
        panic!("the reasoning item is kept")
    };
    assert_eq!(
        reasoning.get("type").and_then(Json::as_str),
        Some("reasoning")
    );
    assert!(reasoning.contains_key("id"));
    let Some(Json::Object(message)) = items.get(1) else {
        panic!("the message item is kept")
    };
    assert_eq!(message.get("type").and_then(Json::as_str), Some("message"));
    assert_eq!(message.get("role").and_then(Json::as_str), Some("user"));
}

fn assert_status_dropped(items: &[Json]) {
    for item in items {
        let Json::Object(fields) = item else {
            panic!("every transformed item is an object")
        };
        assert!(!fields.contains_key("status"));
    }
}

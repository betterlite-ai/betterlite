use betterlite_azure::responses::request::transform_request;
use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::credentials::error::CredentialsError;
use crate::responses::auth::{EnvironmentRequest, validate_environment};
use crate::responses::tests::support::{FixtureSources, accessors, environment, headers, object};

pub(super) fn responses_auth() {
    explicit_key_headers();
    host_specific_header();
    entra_fallback();
    key_resolution();
    header_follows_base();
    missing_credentials();
    inherited_request_filtering();
}

fn explicit_key_headers() {
    let caller = headers(&[("X-Caller", "1"), ("Authorization", "Bearer stale")]);
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let key = SecretString::from("case-key".to_owned());
    let validated = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: accessors(None, None, None, None),
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: Some(&key),
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
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
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let key = SecretString::from("case-key".to_owned());
    let validated = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: accessors(None, None, None, None),
        api_base: Some("https://res.openai.azure.com"),
        api_key: Some(&key),
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        validated.get("api-key").map(String::as_str),
        Some("case-key")
    );
    assert_eq!(validated.get("Authorization"), None);

    let bearer = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: accessors(None, None, None, None),
        api_base: Some("https://api.example.com"),
        api_key: Some(&key),
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        bearer.get("Authorization").map(String::as_str),
        Some("Bearer case-key")
    );
    assert_eq!(bearer.get("api-key"), None);
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
        accessors: accessors(None, None, None, None),
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
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let env_key = SecretString::from("env-key".to_owned());
    let from_environment = accessors(None, None, None, Some(&env_key));
    let resolved = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: from_environment,
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(resolved.get("api-key").map(String::as_str), Some("env-key"));
}

fn layered_key() {
    let caller = headers(&[]);
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let env_key = SecretString::from("env-key".to_owned());
    let global_key = SecretString::from("global-key".to_owned());
    let explicit_key = SecretString::from("explicit-key".to_owned());
    let layered = accessors(None, Some(&global_key), None, Some(&env_key));
    let from_global = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: layered,
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        from_global.get("api-key").map(String::as_str),
        Some("global-key")
    );
    let from_explicit = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: layered,
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: Some(&explicit_key),
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        from_explicit.get("api-key").map(String::as_str),
        Some("explicit-key")
    );
}

fn header_follows_base() {
    let caller = headers(&[]);
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let key = SecretString::from("case-key".to_owned());
    let global = accessors(Some("https://res.services.ai.azure.com"), None, None, None);
    let validated = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: global,
        api_base: None,
        api_key: Some(&key),
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        validated.get("api-key").map(String::as_str),
        Some("case-key")
    );

    let foreign_global = accessors(Some("https://api.example.com"), None, None, None);
    let explicit = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: foreign_global,
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: Some(&key),
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        explicit.get("api-key").map(String::as_str),
        Some("case-key")
    );
}

fn missing_credentials() {
    let caller = headers(&[]);
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let error = validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: accessors(None, None, None, None),
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
    for item in items {
        let Json::Object(fields) = item else {
            panic!("every transformed item is an object")
        };
        assert!(!fields.contains_key("status"));
    }
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

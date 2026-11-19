use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::PassthroughUrlRequest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::error::CredentialsError;
use crate::passthrough::auth::{ValidateEnvironmentRequest, validate_environment};
use crate::passthrough::error::FoundryPassthroughError;
use crate::passthrough::tests::{Offline, build_plan, object, parameters, target};

use super::RES;

pub(super) fn auth_selection() {
    missing_api_base_names_the_environment_variable();
    explicit_values_win_over_global_and_environment();
    environment_values_are_the_last_fallback();
    api_key_headers_follow_the_base_host();
    auth_headers_replace_a_caller_header_of_the_same_name();
    empty_keys_fall_to_entra_tokens();
    a_missing_key_and_token_is_missing_credentials();
    typed_entra_params_name_the_field();
    stream_truthiness_matches_python();
}

fn missing_api_base_names_the_environment_variable() {
    let plan = build_plan(None, None, None, None, Object::new());
    let request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: target("gpt-4o", "chat/completions"),
        request_query_params: None,
        litellm_params: &plan.litellm_params,
    };
    let error = plan.get_complete_url(&request).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Azure AI api base not found: set `api_base` on the deployment or AZURE_AI_API_BASE"
    );
    assert!(matches!(error, FoundryPassthroughError::MissingApiBase));
}

fn explicit_values_win_over_global_and_environment() {
    let plan = build_plan(
        Some("https://global.test"),
        Some("global-key"),
        Some("https://env.test"),
        Some("env-key"),
        Object::new(),
    );
    assert_eq!(
        plan.accessors().api_base(None).as_deref(),
        Some("https://global.test")
    );
    assert_eq!(
        plan.accessors()
            .api_base(Some("https://explicit.test"))
            .as_deref(),
        Some("https://explicit.test")
    );
    assert_eq!(
        plan.accessors().api_base(Some("")),
        Some(String::new()),
        "an explicit empty base is used as given"
    );
    assert_eq!(
        plan.accessors()
            .api_key(None)
            .map(|secret| secret.expose_secret().to_owned()),
        Some(String::from("global-key"))
    );
    let explicit_key = SecretString::from("explicit-key");
    assert_eq!(
        plan.accessors()
            .api_key(Some(&explicit_key))
            .map(|secret| secret.expose_secret().to_owned()),
        Some(String::from("explicit-key"))
    );
}

fn environment_values_are_the_last_fallback() {
    let plan = build_plan(
        None,
        None,
        Some("https://env.test"),
        Some("env-key"),
        Object::new(),
    );
    assert_eq!(
        plan.accessors().api_base(None).as_deref(),
        Some("https://env.test")
    );
    assert_eq!(
        plan.accessors()
            .api_key(None)
            .map(|secret| secret.expose_secret().to_owned()),
        Some(String::from("env-key"))
    );
    let absent = build_plan(None, None, None, None, Object::new());
    assert_eq!(absent.accessors().api_base(None), None);
    assert_eq!(
        absent
            .accessors()
            .api_key(None)
            .map(|secret| secret.expose_secret().to_owned()),
        None
    );
}

fn auth_headers(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    headers: Headers,
) -> Result<Headers, CredentialsError> {
    let environment = BTreeMap::new();
    let sources = Offline;
    validate_environment(ValidateEnvironmentRequest {
        headers,
        api_key,
        params: None,
        api_base,
        environment: &environment,
        refresh_enabled: false,
        sources: &sources,
    })
}

fn api_key_headers_follow_the_base_host() {
    let key = SecretString::from("host-key");
    let foundry = auth_headers(Some(RES), Some(&key), BTreeMap::new()).unwrap();
    assert_eq!(foundry.get("api-key").map(String::as_str), Some("host-key"));
    let openai = auth_headers(
        Some("https://company.openai.azure.com"),
        Some(&key),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(openai.get("api-key").map(String::as_str), Some("host-key"));
    let no_base = auth_headers(None, Some(&key), BTreeMap::new()).unwrap();
    assert_eq!(
        no_base.get("Authorization").map(String::as_str),
        Some("Bearer host-key")
    );
    let other = auth_headers(Some("https://ai.test/v1"), Some(&key), BTreeMap::new()).unwrap();
    assert_eq!(
        other.get("Authorization").map(String::as_str),
        Some("Bearer host-key")
    );
}

fn auth_headers_replace_a_caller_header_of_the_same_name() {
    let key = SecretString::from("foundry-key");
    let caller = BTreeMap::from([
        (String::from("api-key"), String::from("caller")),
        (String::from("x-extra"), String::from("kept")),
    ]);
    let headers = auth_headers(Some(RES), Some(&key), caller).unwrap();
    assert_eq!(
        headers.get("api-key").map(String::as_str),
        Some("foundry-key")
    );
    assert_eq!(headers.get("x-extra").map(String::as_str), Some("kept"));
    assert_eq!(headers.len(), 2);
}

fn empty_keys_fall_to_entra_tokens() {
    let environment = BTreeMap::from([(
        String::from("AZURE_AD_TOKEN"),
        String::from("fixture-entra-token"),
    )]);
    let sources = Offline;
    let empty_key = SecretString::from("");
    let headers = validate_environment(ValidateEnvironmentRequest {
        headers: BTreeMap::new(),
        api_key: Some(&empty_key),
        params: None,
        api_base: Some(RES),
        environment: &environment,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer fixture-entra-token")
    );
    assert_eq!(headers.get("api-key"), None);
}

fn a_missing_key_and_token_is_missing_credentials() {
    let key = SecretString::from("");
    let error = auth_headers(Some(RES), Some(&key), BTreeMap::new()).unwrap_err();
    assert!(error.to_string().contains("AZURE_AI_API_KEY"), "{error}");
    assert!(matches!(error, CredentialsError::MissingCredentials { .. }));
}

fn typed_entra_params_name_the_field() {
    let params = parameters(r#"{"tenant_id": 5}"#);
    let environment = BTreeMap::new();
    let sources = Offline;
    let error = validate_environment(ValidateEnvironmentRequest {
        headers: BTreeMap::new(),
        api_key: None,
        params: Some(&params),
        api_base: Some(RES),
        environment: &environment,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "litellm_params.tenant_id must be a string"
    );
    assert!(matches!(
        error,
        CredentialsError::InvalidParam { field: "tenant_id" }
    ));
}

fn stream_truthiness_matches_python() {
    let plan = build_plan(None, None, None, None, Object::new());
    for (body, expected) in [
        ("{}", false),
        (r#"{"stream": true}"#, true),
        (r#"{"stream": false}"#, false),
        (r#"{"stream": null}"#, false),
        (r#"{"stream": 0}"#, false),
        (r#"{"stream": 0.0}"#, false),
        (r#"{"stream": 1}"#, true),
        (r#"{"stream": -1}"#, true),
        (r#"{"stream": ""}"#, false),
        (r#"{"stream": "false"}"#, true),
        (r#"{"stream": []}"#, false),
        (r#"{"stream": [false]}"#, true),
        (r#"{"stream": {}}"#, false),
        (r#"{"stream": {"a": 0}}"#, true),
    ] {
        assert_eq!(
            plan.is_streaming_request("chat/completions", &object(body)),
            expected,
            "{body}"
        );
    }
}

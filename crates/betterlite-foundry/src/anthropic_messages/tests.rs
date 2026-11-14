#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_prompts::arguments::json::Object;
use rstest::rstest;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use super::environment::{
    EnvironmentRequest, Headers, ParamsKey, TokenSource, validate_anthropic_messages_environment,
};
use super::environment_error::EnvironmentError;
use super::transform::transform_anthropic_messages_request;
use super::url::get_complete_url;
use super::{CUSTOM_LLM_PROVIDER, SHOULD_STRIP_BILLING_METADATA};

const SENTINEL: &str = "sentinel-secret-value";

#[derive(Debug, Error)]
#[error("fixture beta failure")]
struct BetaFailure;

struct NoSources;

fn refuse<T>(kind: &str) -> Result<T, SourceFailure> {
    Err(SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: format!("no {kind} source is configured"),
    })
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("client secret")
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("username and password")
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("SDK refresh")
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        refuse("OIDC")
    }
}

fn settings(token: Option<&str>, client_secret: Option<&str>) -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: token.map(SecretString::from),
        tenant_id: client_secret.map(|_| "tenant".to_owned()),
        client_id: client_secret.map(|_| "client".to_owned()),
        client_secret: client_secret.map(SecretString::from),
        azure_scope: "https://cognitiveservices.azure.com/.default".to_owned(),
        azure_username: None,
        azure_password: None,
    }
}

// Stands in for B960: it must see the defaults already applied, and a header can make it fail
fn fixture_betas(mut headers: Headers) -> Result<Headers, BetaFailure> {
    assert!(headers.contains_key("anthropic-version") && headers.contains_key("content-type"));
    if headers.contains_key("fail-betas") {
        return Err(BetaFailure);
    }
    headers.insert("anthropic-beta".to_owned(), "fixture-beta".to_owned());
    Ok(headers)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    api_base: Option<String>,
    environment: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    params: Option<String>,
    argument_key: Option<String>,
    configured_key: Option<String>,
    token: Option<String>,
    api_base: Option<String>,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Outcome {
    headers: Option<Headers>,
    api_base: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Projection {
    custom_llm_provider: String,
    should_strip_billing_metadata: bool,
    request: Object,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Scenario {
    Url {
        calls: Vec<UrlCall>,
        expected: Vec<String>,
    },
    Environment {
        calls: Vec<EnvironmentCall>,
        expected: Vec<Outcome>,
    },
    Transform {
        request: Object,
        expected: Object,
    },
    Projection {
        request: Object,
        expected: Projection,
    },
}

fn url(call: &UrlCall) -> String {
    match get_complete_url(call.api_base.as_deref(), call.environment.as_deref()) {
        Ok(complete) => complete,
        Err(error) => error.to_string(),
    }
}

fn fixture_request<'data>(
    call: EnvironmentCall,
    params_key: ParamsKey<'data>,
    argument: Option<&'data SecretString>,
    configured: Option<&'data SecretString>,
    credentials: &'data DeploymentSettings,
) -> EnvironmentRequest<'data> {
    EnvironmentRequest {
        headers: call.headers,
        params_key,
        argument_key: argument,
        configured_key: configured,
        token: TokenSource {
            credentials,
            refresh_enabled: false,
            provider: None,
            sources: &NoSources,
        },
        api_base: call.api_base,
    }
}

// "dict_absent", "dict_null", "dict:KEY", "typed" and "typed:KEY" name the shape LiteLLM received
fn environment(call: EnvironmentCall) -> Outcome {
    let spec = call.params.unwrap_or_else(|| "dict_absent".to_owned());
    let (shape, value) = spec
        .split_once(':')
        .map_or((spec.as_str(), None), |(name, text)| {
            (name, Some(SecretString::from(text)))
        });
    let params_key = match (shape, value.as_ref()) {
        ("dict_absent", None) => ParamsKey::DictAbsent,
        ("dict_null", None) => ParamsKey::DictNull,
        ("dict", Some(secret)) => ParamsKey::Dict(secret),
        ("typed", typed) => ParamsKey::Typed(typed),
        _ => panic!("unknown params spec {spec}"),
    };
    let argument = call.argument_key.as_deref().map(SecretString::from);
    let configured = call.configured_key.as_deref().map(SecretString::from);
    let credentials = settings(call.token.as_deref(), None);
    let request = fixture_request(
        call,
        params_key,
        argument.as_ref(),
        configured.as_ref(),
        &credentials,
    );
    match validate_anthropic_messages_environment(request, fixture_betas) {
        Ok((headers, api_base)) => Outcome {
            headers: Some(headers),
            api_base,
            error: None,
        },
        Err(error) => Outcome {
            error: Some(error.to_string()),
            ..Outcome::default()
        },
    }
}

#[rstest]
#[case::messages_url_normalization(
    r#"{"url": {"calls": [
        {"api_base": "https://r.services.ai.azure.com/anthropic"},
        {"api_base": "https://r.services.ai.azure.com/anthropic///"},
        {"api_base": "https://r.services.ai.azure.com"},
        {"api_base": "https://r.services.ai.azure.com/v1/messages/"},
        {"api_base": "https://r.services.ai.azure.com/anthropic/v1/messages"},
        {"api_base": "https://r.services.ai.azure.com/anthropic/v1/other"},
        {"api_base": "https://r.services.ai.azure.com/anthropic?x=1"},
        {"api_base": "https://r.services.ai.azure.com/anthropic/v1/messages?x=1"},
        {"api_base": "https://r.services.ai.azure.com/other?next=/anthropic/v2"},
        {"api_base": "https://r.services.ai.azure.com/anthropicfoo/x"},
        {"api_base": "", "environment": "https://env.services.ai.azure.com/anthropic/"},
        {"api_base": "https://explicit/anthropic", "environment": "https://env/anthropic"},
        {"environment": "https://env.services.ai.azure.com"},
        {"api_base": "", "environment": ""},
        {}
    ], "expected": [
        "https://r.services.ai.azure.com/anthropic/v1/messages",
        "https://r.services.ai.azure.com/anthropic/v1/messages",
        "https://r.services.ai.azure.com/anthropic/v1/messages",
        "https://r.services.ai.azure.com/v1/messages",
        "https://r.services.ai.azure.com/anthropic/v1/messages",
        "https://r.services.ai.azure.com/anthropic/v1/messages",
        "https://r.services.ai.azure.com/anthropic/v1/messages",
        "https://r.services.ai.azure.com/anthropic/v1/messages",
        "https://r.services.ai.azure.com/other?next=/anthropic/v1/messages",
        "https://r.services.ai.azure.com/anthropic/v1/messages",
        "https://env.services.ai.azure.com/anthropic/v1/messages",
        "https://explicit/anthropic/v1/messages",
        "https://env.services.ai.azure.com/anthropic/v1/messages",
        "/anthropic/v1/messages",
        "Missing Azure API Base - Please set `api_base` or `AZURE_API_BASE` environment variable. Expected format: https://<resource-name>.services.ai.azure.com/anthropic"
    ]}}"#
)]
#[case::x_api_key_rename(
    r#"{"environment": {"calls": [
        {"headers": {"api-key": "header-key"}},
        {"headers": {"api-key": "header-key"}, "argument_key": "arg-key"},
        {"headers": {}, "argument_key": "arg-key"},
        {"headers": {"api-key": "a", "x-api-key": "x"}},
        {"headers": {"api-key": "a", "x-api-key": ""}},
        {"headers": {"Api-Key": "cased"}, "argument_key": "arg-key"},
        {"params": "dict_null", "argument_key": "arg-key"},
        {"params": "dict_null", "argument_key": "arg-key", "configured_key": "cfg-key"},
        {"params": "dict:param-key", "argument_key": "arg-key"},
        {"params": "dict:", "argument_key": "arg-key", "configured_key": "cfg-key"},
        {"params": "typed", "argument_key": "arg-key"},
        {"params": "typed:", "argument_key": "arg-key"},
        {"params": "typed:typed-key", "argument_key": "arg-key"},
        {"configured_key": "cfg-key"},
        {"token": "ad-token"}
    ], "expected": [
        {"headers": {"x-api-key": "header-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "header-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "arg-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"api-key": "a", "x-api-key": "x", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"api-key": "a", "x-api-key": "", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"Api-Key": "cased", "x-api-key": "arg-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "cfg-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "param-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "cfg-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "arg-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "arg-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "typed-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"x-api-key": "cfg-key", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"Authorization": "Bearer ad-token", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}}
    ]}}"#
)]
#[case::anthropic_version_default(
    r#"{"environment": {"calls": [
        {"headers": {}, "api_base": "https://base"},
        {"headers": {"anthropic-version": "", "content-type": ""}},
        {"headers": {"Anthropic-Version": "2024-01-01", "Content-Type": "text/plain"}},
        {"headers": {"anthropic-version": "2020-01-01"}},
        {"headers": {"content-type": "text/plain"}, "api_base": "https://base/anthropic"},
        {"headers": {"fail-betas": "1"}}
    ], "expected": [
        {"headers": {"anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}, "api_base": "https://base"},
        {"headers": {"anthropic-version": "", "content-type": "", "anthropic-beta": "fixture-beta"}},
        {"headers": {"Anthropic-Version": "2024-01-01", "Content-Type": "text/plain", "anthropic-version": "2023-06-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"anthropic-version": "2020-01-01", "content-type": "application/json", "anthropic-beta": "fixture-beta"}},
        {"headers": {"anthropic-version": "2023-06-01", "content-type": "text/plain", "anthropic-beta": "fixture-beta"}, "api_base": "https://base/anthropic"},
        {"error": "fixture beta failure"}
    ]}}"#
)]
#[case::cache_control_scope_removal(
    r#"{"transform": {"request": {
        "system": [
            {"type": "text", "text": "a", "cache_control": {"type": "ephemeral", "ttl": "1h", "scope": "global", "extra": "kept"}},
            {"type": "text", "text": "b", "cache_control": {"scope": "global"}},
            {"type": "text", "text": "c", "cache_control": "scope"},
            {"type": "text", "text": "d", "cache_control": null},
            {"type": "text", "text": "e"},
            "plain"
        ],
        "messages": [
            {"role": "user", "content": [
                {"type": "text", "text": "x", "cache_control": {"type": "ephemeral", "scope": "global"}},
                {"type": "tool_result", "tool_use_id": "t", "content": [
                    {"type": "text", "text": "nested", "cache_control": {"type": "ephemeral", "scope": "global"}}
                ]},
                {"type": "text", "text": "y", "cache_control": ["scope"]}
            ]},
            {"role": "assistant", "content": "string content", "cache_control": {"scope": "global"}},
            "not a message"
        ],
        "tools": [{"name": "tool", "cache_control": {"type": "ephemeral", "scope": "global"}}]
    }, "expected": {
        "system": [
            {"type": "text", "text": "a", "cache_control": {"type": "ephemeral", "ttl": "1h", "extra": "kept"}},
            {"type": "text", "text": "b", "cache_control": {}},
            {"type": "text", "text": "c", "cache_control": "scope"},
            {"type": "text", "text": "d", "cache_control": null},
            {"type": "text", "text": "e"},
            "plain"
        ],
        "messages": [
            {"role": "user", "content": [
                {"type": "text", "text": "x", "cache_control": {"type": "ephemeral"}},
                {"type": "tool_result", "tool_use_id": "t", "content": [
                    {"type": "text", "text": "nested", "cache_control": {"type": "ephemeral", "scope": "global"}}
                ]},
                {"type": "text", "text": "y", "cache_control": ["scope"]}
            ]},
            {"role": "assistant", "content": "string content", "cache_control": {"scope": "global"}},
            "not a message"
        ],
        "tools": [{"name": "tool", "cache_control": {"type": "ephemeral", "scope": "global"}}]
    }}}"#
)]
#[case::system_role_normalization(
    r#"{"transform": {"request": {
        "model": "claude-sonnet-4-5",
        "system": [{"type": "text", "text": "be brief", "cache_control": {"type": "ephemeral", "scope": "global"}}],
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
    }, "expected": {
        "model": "claude-sonnet-4-5",
        "system": [{"type": "text", "text": "be brief", "cache_control": {"type": "ephemeral"}}],
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
    }}}"#
)]
#[case::native_body_projection(
    r#"{"projection": {"request": {
        "model": "claude-sonnet-4-5",
        "system": "plain system",
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral", "scope": "global"}}]}]
    }, "expected": {
        "custom_llm_provider": "azure_ai",
        "should_strip_billing_metadata": true,
        "request": {
            "model": "claude-sonnet-4-5",
            "system": "plain system",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}]}]
        }
    }}}"#
)]
fn anthropic_messages_cases(#[case] source: &str) {
    match serde_json::from_str::<Scenario>(source).unwrap() {
        Scenario::Url { calls, expected } => {
            let actual: Vec<String> = calls.iter().map(url).collect();
            assert_eq!(actual, expected);
        }
        Scenario::Environment { calls, expected } => {
            let actual: Vec<Outcome> = calls.into_iter().map(environment).collect();
            assert_eq!(actual, expected);
        }
        Scenario::Transform {
            mut request,
            expected,
        } => {
            transform_anthropic_messages_request(&mut request);
            assert_eq!(request, expected);
        }
        Scenario::Projection {
            mut request,
            expected,
        } => {
            transform_anthropic_messages_request(&mut request);
            let actual = Projection {
                custom_llm_provider: CUSTOM_LLM_PROVIDER.to_owned(),
                should_strip_billing_metadata: SHOULD_STRIP_BILLING_METADATA,
                request,
            };
            assert_eq!(actual, expected);
        }
    }
}

fn key_of(outcome: &Outcome, name: &str) -> Option<String> {
    outcome
        .headers
        .as_ref()
        .and_then(|headers| headers.get(name))
        .cloned()
}

#[test]
fn anthropic_messages_credential_identity_isolation() {
    let from_header = environment(EnvironmentCall {
        headers: BTreeMap::from([("api-key".to_owned(), "identity-a-key".to_owned())]),
        ..EnvironmentCall::default()
    });
    let from_argument = environment(EnvironmentCall {
        argument_key: Some("identity-b-key".to_owned()),
        ..EnvironmentCall::default()
    });
    let from_token = environment(EnvironmentCall {
        token: Some("identity-c-token".to_owned()),
        ..EnvironmentCall::default()
    });
    let anonymous = environment(EnvironmentCall::default());
    assert_eq!(
        key_of(&from_header, "x-api-key").as_deref(),
        Some("identity-a-key")
    );
    assert_eq!(
        key_of(&from_argument, "x-api-key").as_deref(),
        Some("identity-b-key")
    );
    assert_eq!(key_of(&from_argument, "Authorization"), None);
    assert_eq!(
        key_of(&from_token, "Authorization").as_deref(),
        Some("Bearer identity-c-token")
    );
    assert_eq!(key_of(&from_token, "x-api-key"), None);
    assert_eq!(key_of(&anonymous, "x-api-key"), None);
    assert_eq!(key_of(&anonymous, "Authorization"), None);
    let repeated = environment(EnvironmentCall {
        argument_key: Some("identity-b-key".to_owned()),
        ..EnvironmentCall::default()
    });
    assert_eq!(repeated, from_argument);
}

fn secret_request<'data>(
    credentials: &'data DeploymentSettings,
    headers: Headers,
    argument_key: Option<&'data SecretString>,
) -> EnvironmentRequest<'data> {
    EnvironmentRequest {
        headers,
        params_key: ParamsKey::DictAbsent,
        argument_key,
        configured_key: None,
        token: TokenSource {
            credentials,
            refresh_enabled: false,
            provider: None,
            sources: &NoSources,
        },
        api_base: None,
    }
}

#[test]
fn anthropic_messages_secret_safe_failure() {
    let secret = SecretString::from(SENTINEL);
    for params_key in [ParamsKey::Dict(&secret), ParamsKey::Typed(Some(&secret))] {
        assert!(!format!("{params_key:?}").contains(SENTINEL));
    }
    let credentials = settings(None, Some(SENTINEL));
    let failing_betas = BTreeMap::from([("fail-betas".to_owned(), "1".to_owned())]);
    let beta_error = validate_anthropic_messages_environment(
        secret_request(&credentials, failing_betas, Some(&secret)),
        fixture_betas,
    )
    .unwrap_err();
    assert!(matches!(beta_error, EnvironmentError::Betas(BetaFailure)));
    let source_error = validate_anthropic_messages_environment(
        secret_request(&credentials, Headers::new(), None),
        fixture_betas,
    )
    .unwrap_err();
    assert!(matches!(source_error, EnvironmentError::Token(_)));
    let missing = get_complete_url(None, None).unwrap_err();
    for text in [
        format!("{beta_error} {beta_error:?}"),
        format!("{source_error} {source_error:?}"),
        format!("{missing} {missing:?}"),
    ] {
        assert!(!text.contains(SENTINEL));
    }
}

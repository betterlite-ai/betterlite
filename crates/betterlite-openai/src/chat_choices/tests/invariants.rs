#![cfg(test)]

use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::openai_credentials::Settings as CredentialSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use secrecy::{ExposeSecret, SecretString};
use tokio::runtime::Builder;
use tokio::task::yield_now;
use url::Url;

use crate::chat_choices::endpoint::{
    ModelListRequest, ModelListTransport, get_models, validate_environment,
};
use crate::credentials::{
    ExplicitCredentials, LiteLlmGlobals, OpenAiCredentials, get_openai_credentials,
};
use crate::openai_base_config::Headers;

fn settings(api_base: &str) -> CredentialSettings {
    CredentialSettings {
        api_base: api_base.to_owned(),
        organization: None,
    }
}

fn credentials(configured_key: &str, base: &str) -> OpenAiCredentials {
    get_openai_credentials(
        ExplicitCredentials {
            api_key: Some(SecretString::from(configured_key)),
            api_base: Some(base.to_owned()),
            organization: None,
        },
        LiteLlmGlobals::default(),
        &settings("https://environment.example/v1"),
        None,
    )
}

fn key_and_base(resolved: &OpenAiCredentials) -> (Option<&str>, &str) {
    (
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        resolved.api_base.as_str(),
    )
}

#[test]
fn openai_chat_choices_credential_identity_isolation() {
    let first = credentials("key-a", "https://a.example/v1");
    let second = credentials("key-b", "https://b.example/v1");
    assert_eq!(
        key_and_base(&first),
        (Some("key-a"), "https://a.example/v1")
    );
    assert_eq!(
        key_and_base(&second),
        (Some("key-b"), "https://b.example/v1")
    );

    let first_headers = validate_environment(Some(&SecretString::from("key-a")), &Headers::new());
    let second_headers = validate_environment(Some(&SecretString::from("key-b")), &Headers::new());
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some("Bearer key-a")
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer key-b")
    );

    // resolving the same identity again never picks up the other one's values
    let again = credentials("key-a", "https://a.example/v1");
    assert_eq!(key_and_base(&again), key_and_base(&first));
}

struct Failing;

impl ModelListTransport for Failing {
    async fn get(
        &self,
        _url: &Url,
        _headers: &HeaderMap,
        _deadline: Duration,
    ) -> Result<UpstreamResponse, TransportError> {
        yield_now().await;
        Ok(UpstreamResponse {
            status: 500,
            headers: HeaderMap::new(),
            body: Vec::new(),
        })
    }
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn request<'base>(api_base: &'base str, key: &str) -> ModelListRequest<'base> {
    ModelListRequest {
        api_key: Some(SecretString::from(key)),
        api_base: Some(api_base),
        environment_api_key: None,
        deadline: Duration::from_secs(600),
    }
}

#[test]
fn openai_chat_choices_secret_safe_failure() {
    let secret = "sk-fixture-secret";
    let resolved = credentials(secret, "https://a.example/v1");
    let debugged = format!("{resolved:?}");
    assert!(!debugged.contains(secret), "{debugged}");

    let error = block_on(get_models(
        &Failing,
        &request("https://a.example/v1", secret),
    ))
    .unwrap_err();
    let reported = format!("{error} {error:?}");
    assert!(!reported.contains(secret), "{reported}");
    let invalid_error = block_on(get_models(&Failing, &request("not a url", secret))).unwrap_err();
    let invalid = format!("{invalid_error} {invalid_error:?}");
    assert!(!invalid.contains(secret), "{invalid}");
}

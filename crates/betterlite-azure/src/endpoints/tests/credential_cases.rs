use secrecy::SecretString;

use super::support::{
    Cache, Inputs, KeyedCache, Run, Script, get_client, initialize, initialize_result,
};
use crate::credentials::headers::{ClientAuthParams, get_azure_request_auth_headers};
use crate::credentials::source::{SourceFailure, TokenReply};
use crate::endpoints::get_client::ClientCache as _;
use crate::endpoints::identity::client_identity;
use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::{AzureClientInputs, ClientKind};

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn identity_provider_token(value: &AzureClientInputs<'_>) -> SecretString {
    let reply = value.token_provider.as_ref().unwrap().0.token().unwrap();
    match reply {
        TokenReply::Text(token) => token,
        TokenReply::NonText { type_name } => panic!("{type_name}"),
    }
}

fn authorization(token: &SecretString) -> Vec<(String, String)> {
    get_azure_request_auth_headers(
        &[],
        &ClientAuthParams {
            azure_ad_token: Some(token.clone()),
            azure_ad_token_provider: None,
            api_key: None,
        },
    )
    .unwrap()
}

fn identity_headers() {
    let first_secret = secret("client-secret-a");
    let mut first = Inputs::default()
        .with_tenant("tenant-a")
        .with_client_id("client-a")
        .with_client_secret(&first_secret);
    first.set_script("client_secret", Script::Token("token-a".to_owned()));
    let second_secret = secret("client-secret-b");
    let mut second = Inputs::default()
        .with_tenant("tenant-b")
        .with_client_id("client-b")
        .with_client_secret(&second_secret);
    second.set_script("client_secret", Script::Token("token-b".to_owned()));
    let first_value = initialize(&first, &Run::default());
    let second_value = initialize(&second, &Run::default());
    assert_eq!(
        authorization(&identity_provider_token(&first_value)),
        vec![("Authorization".to_owned(), "Bearer token-a".to_owned())]
    );
    assert_eq!(
        authorization(&identity_provider_token(&second_value)),
        vec![("Authorization".to_owned(), "Bearer token-b".to_owned())]
    );
}

fn identity_cache_entries() {
    let first = Inputs::default().with_client_secret(&secret("client-secret-a"));
    let second = Inputs::default().with_client_secret(&secret("client-secret-b"));
    let key = secret("fixture-api-key");
    let mut cache = KeyedCache::empty();
    let run = Run::for_client().with_key(&key);
    let first_outcome = get_client(&first, &run, &mut cache).unwrap();
    let second_outcome = get_client(&second, &run, &mut cache).unwrap();
    assert_ne!(first_outcome.identity, second_outcome.identity);
    assert_eq!(
        cache.get(&first_outcome.identity),
        Some(ClientKind::AzureOpenAi)
    );
    assert_eq!(
        cache.get(&second_outcome.identity),
        Some(ClientKind::AzureOpenAi)
    );
    let other_key = secret("other-api-key");
    let third = client_identity(Some(&other_key), false, &Inputs::default().params, None);
    assert_eq!(cache.get(&third), None);
}

pub(crate) fn credential_identity_isolation() {
    identity_headers();
    identity_cache_entries();
}

fn secret_inputs() -> Inputs {
    Inputs::default()
        .with_ad_token(&secret("fixture-ad-token"))
        .with_client_secret(&secret("fixture-client-secret"))
        .with_password(&secret("fixture-password"))
}

fn assert_no_secrets(rendered: &str) {
    for text in [
        "fixture-ad-token",
        "fixture-client-secret",
        "fixture-password",
        "fixture-api-key",
    ] {
        assert!(!rendered.contains(text), "{text} leaked");
    }
}

fn inputs_debug_redacts() {
    let inputs = secret_inputs();
    let key = secret("fixture-api-key");
    let value = initialize(&inputs, &Run::default().with_key(&key));
    assert_no_secrets(&format!("{value:?}"));
}

fn error_debug_redacts() {
    let mut inputs = Inputs::default()
        .with_ad_token(&secret("oidc/fixture-ad-token"))
        .with_client_secret(&secret("fixture-client-secret"));
    inputs.set_script(
        "oidc",
        Script::Raise {
            class: "AzureOpenAIError".to_owned(),
            message: "OIDC token could not be retrieved".to_owned(),
        },
    );
    let error = initialize_result(&inputs, &Run::default()).unwrap_err();
    assert_eq!(error.class(), "AzureOpenAIError");
    assert_no_secrets(&format!("{error}"));
    assert_no_secrets(&format!("{error:?}"));
}

fn error_reports_status() {
    let rejected = EndpointError::Source(SourceFailure {
        class: "AzureOpenAIError".to_owned(),
        status: Some(401),
        message: "OIDC token could not be retrieved".to_owned(),
    });
    assert_eq!(rejected.status(), Some(401));
    assert_eq!(EndpointError::MissingApiVersion.status(), None);
    assert_eq!(EndpointError::MissingApiVersion.class(), "KeyError");
}

fn outcome_debug_redacts() {
    let inputs = secret_inputs();
    let key = secret("fixture-api-key");
    let mut cache = Cache::empty();
    let run = Run::for_client().with_key(&key);
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert_no_secrets(&format!("{outcome:?}"));
    assert!(
        !outcome
            .identity
            .provider_identity
            .contains("fixture-client-secret")
    );
    assert!(
        !outcome
            .identity
            .provider_identity
            .contains("fixture-password")
    );
}

pub(crate) fn secret_safe_failure() {
    inputs_debug_redacts();
    error_debug_redacts();
    error_reports_status();
    outcome_debug_redacts();
}

use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use secrecy::SecretString;
use serde::Deserialize;
use url::form_urlencoded::byte_serialize;

use super::support::{Env, HANG, body_of, header, implicit, oidc_settings};
use crate::hub_oauth::error::OidcError;
use crate::hub_token::{HubToken, HubTokenError, HubTokenResolver};

#[derive(Debug, Deserialize, PartialEq)]
struct ExchangeRequest {
    grant_type: String,
    subject_token_type: String,
    subject_token: String,
    resource: String,
}

fn exchange_request(request: &str) -> ExchangeRequest {
    serde_json::from_str(body_of(request)).unwrap()
}

fn supplied_token_settings() -> OidcSettings {
    let mut oidc = oidc_settings(Some("org/repo"));
    oidc.id_token = Some(SecretString::from("the-jwt"));
    oidc
}

fn github_lookup(env: &Env) -> HubTokenResolver {
    let mut oidc = oidc_settings(Some("user"));
    oidc.github_actions = true;
    oidc.request_url = Some(format!("{}/github?api=1", env.server.endpoint));
    oidc.request_token = Some(SecretString::from("request-token"));
    env.lookup(env.hub(), oidc)
}

pub(super) async fn exchanges_the_env_id_token() {
    let env = Env::new().await;
    env.server
        .answer(200, r#"{"access_token":"hf_one","expires_in":1000}"#);
    let mut hub = env.hub();
    hub.env_token = Some(SecretString::from("from-env"));
    let lookup = env.lookup(hub, supplied_token_settings());

    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_one");
    let requests = env.server.requests();
    let request = requests.first().unwrap();
    assert!(request.starts_with("POST /oauth/token HTTP/1.1\r\n"));
    assert_eq!(header(request, "content-type"), Some("application/json"));
    assert_eq!(
        exchange_request(request),
        ExchangeRequest {
            grant_type: "urn:ietf:params:oauth:grant-type:token-exchange".to_owned(),
            subject_token_type: "urn:ietf:params:oauth:token-type:id_token".to_owned(),
            subject_token: "the-jwt".to_owned(),
            resource: "org/repo".to_owned(),
        }
    );
}

pub(super) async fn caches_a_supplied_token_for_its_whole_lifetime() {
    let env = Env::new().await;
    env.server
        .answer(200, r#"{"access_token":"hf_one","expires_in":1000}"#);
    env.server.answer(200, r#"{"access_token":"hf_two"}"#);
    env.server
        .answer(200, r#"{"access_token":"hf_three","expires_in":1}"#);
    let lookup = env.lookup(env.hub(), supplied_token_settings());

    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_one");
    env.clock.advance(999);
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_one");
    assert_eq!(env.server.requests().len(), 1);
    env.clock.advance(1);
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_two");
    assert_eq!(env.server.requests().len(), 2);

    // expires_in defaults to 3600
    env.clock.advance(3599);
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_two");
    env.clock.advance(1);
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_three");
    assert_eq!(env.server.requests().len(), 3);
}

pub(super) async fn explicit_token_makes_no_request() {
    let env = Env::new().await;
    let lookup = env.lookup(env.hub(), supplied_token_settings());
    let token = lookup
        .resolve(&HubToken::Explicit(SecretString::from("given")))
        .await
        .unwrap();
    assert!(token.is_some());
    assert!(env.server.requests().is_empty());
}

pub(super) async fn mints_a_github_actions_token_first() {
    let env = Env::new().await;
    env.server.answer(200, r#"{"value":"minted-1"}"#);
    env.server
        .answer(200, r#"{"access_token":"hf_gh1","expires_in":3600}"#);
    let lookup = github_lookup(&env);

    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_gh1");
    let requests = env.server.requests();
    let minting = requests.first().unwrap();
    let audience: String = byte_serialize(env.server.endpoint.as_bytes()).collect();
    assert!(
        minting.starts_with(&format!(
            "GET /github?api=1&audience={audience} HTTP/1.1\r\n"
        )),
        "{minting}"
    );
    assert_eq!(
        header(minting, "authorization"),
        Some("Bearer request-token")
    );
    let exchange = &requests[1];
    assert_eq!(exchange_request(exchange).subject_token, "minted-1");
}

pub(super) async fn renews_a_minted_token_before_it_ends() {
    let env = Env::new().await;
    for (minted, exchanged) in [
        ("minted-1", r#"{"access_token":"hf_gh1","expires_in":3600}"#),
        ("minted-2", r#"{"access_token":"hf_gh2","expires_in":100}"#),
        ("minted-3", r#"{"access_token":"hf_gh3","expires_in":3600}"#),
    ] {
        env.server
            .answer(200, &format!(r#"{{"value":"{minted}"}}"#));
        env.server.answer(200, exchanged);
    }
    let lookup = github_lookup(&env);

    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_gh1");
    env.clock.advance(3299);
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_gh1");
    assert_eq!(env.server.requests().len(), 2);
    env.clock.advance(1);
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_gh2");
    assert_eq!(env.server.requests().len(), 4);
    // a lifetime shorter than the margin is never cached
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_gh3");
    assert_eq!(env.server.requests().len(), 6);
}

pub(super) async fn unset_resource_falls_through_to_the_env_token() {
    let env = Env::new().await;
    let mut hub = env.hub();
    hub.env_token = Some(SecretString::from("from-env"));
    let lookup = env.lookup(hub, oidc_settings(None));
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "from-env");
    assert!(env.server.requests().is_empty());
}

pub(super) async fn missing_inputs_are_errors_even_with_an_env_token() {
    let env = Env::new().await;
    let mut hub = env.hub();
    hub.env_token = Some(SecretString::from("from-env"));

    let no_id_token = env.lookup(hub.clone(), oidc_settings(Some("org/repo")));
    assert!(matches!(
        implicit(&no_id_token).await,
        Err(HubTokenError::Oidc(OidcError::NoIdToken))
    ));

    let mut github = oidc_settings(Some("org/repo"));
    github.github_actions = true;
    let no_request_url = env.lookup(hub, github);
    assert!(matches!(
        implicit(&no_request_url).await,
        Err(HubTokenError::Oidc(OidcError::GithubRequest))
    ));
    assert!(env.server.requests().is_empty());
}

pub(super) async fn rejected_exchanges_are_errors_and_not_cached() {
    let env = Env::new().await;
    let lookup = env.lookup(env.hub(), supplied_token_settings());
    env.server.answer(401, r#"{"error":"nope"}"#);
    assert!(matches!(
        implicit(&lookup).await,
        Err(HubTokenError::Oidc(OidcError::Status { status: 401, .. }))
    ));
    env.server.answer(200, r#"{"token_type":"bearer"}"#);
    assert!(matches!(
        implicit(&lookup).await,
        Err(HubTokenError::Oidc(OidcError::Body { .. }))
    ));
    env.server.answer(200, r#"{"access_token":"hf_ok"}"#);
    assert_eq!(implicit(&lookup).await.unwrap().unwrap(), "hf_ok");
}

pub(super) async fn exchange_uses_the_download_timeout() {
    let env = Env::new().await;
    env.server.answer(HANG, "");
    let mut hub = env.hub();
    hub.download_timeout = 1;
    let lookup = env.lookup(hub, supplied_token_settings());
    assert!(matches!(
        implicit(&lookup).await,
        Err(HubTokenError::Oidc(OidcError::Transport { .. }))
    ));
}

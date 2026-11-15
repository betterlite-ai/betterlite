#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::errors::CredError;
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::httpx_auth::{Auth, HeaderMap};
use secrecy::{ExposeSecret as _, SecretString};

use super::{AuthorizedCall, ClientCredentialsBearerAuth, Refetch, SourceRefetch};
use crate::mcp::outbound_credentials::SystemWallClock;
use crate::mcp::outbound_credentials::client_credentials::source::{
    ClientCredentialsTokenSource, Tunables,
};
use crate::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome,
};

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

struct ScriptedCall {
    statuses: Mutex<VecDeque<u16>>,
    sent: Mutex<Vec<Option<String>>>,
}

impl ScriptedCall {
    fn new(statuses: Vec<u16>) -> Self {
        Self {
            statuses: Mutex::new(VecDeque::from(statuses)),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn sent_tokens(&self) -> Vec<Option<String>> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AuthorizedCall for ScriptedCall {
    type Output = u16;

    async fn send(&self, auth: &(dyn Auth + Sync)) -> u16 {
        let mut headers = HeaderMap::new();
        auth.apply(&mut headers);
        let value = headers
            .get("authorization")
            .map(|value| String::from_utf8(value.as_bytes().to_vec()).unwrap());
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
        self.statuses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the script has an answer left")
    }

    fn is_unauthorized(&self, output: &u16) -> bool {
        *output == 401
    }
}

struct ScriptedRefetch {
    answers: Mutex<VecDeque<Option<SecretString>>>,
    asked: Mutex<Vec<String>>,
}

impl ScriptedRefetch {
    fn new(answers: Vec<Option<&str>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(
                answers
                    .into_iter()
                    .map(|answer| answer.map(secret))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked_for(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl Refetch for ScriptedRefetch {
    async fn refetch(&self, failed_access_token: &SecretString) -> Option<SecretString> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(failed_access_token.expose_secret().to_owned());
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the script has an answer left")
    }
}

fn auth(token: &str, refetch: Arc<ScriptedRefetch>) -> ClientCredentialsBearerAuth {
    ClientCredentialsBearerAuth::new(&secret(token), refetch, HeaderCarrier::default()).unwrap()
}

#[tokio::test]
async fn a_non_401_answer_is_returned_after_one_send() {
    let refetch = ScriptedRefetch::new(vec![Some("fresh")]);
    let bearer = auth("old", Arc::clone(&refetch));
    let call = ScriptedCall::new(vec![200]);
    assert_eq!(bearer.send_with_retry(&call).await.unwrap(), 200);
    assert_eq!(call.sent_tokens(), vec![Some("Bearer old".to_owned())]);
    assert!(refetch.asked_for().is_empty());
}

#[tokio::test]
async fn a_401_is_retried_once_with_the_fresh_token() {
    let refetch = ScriptedRefetch::new(vec![Some("fresh")]);
    let bearer = auth("old", Arc::clone(&refetch));
    let call = ScriptedCall::new(vec![401, 200]);
    assert_eq!(bearer.send_with_retry(&call).await.unwrap(), 200);
    assert_eq!(
        call.sent_tokens(),
        vec![
            Some("Bearer old".to_owned()),
            Some("Bearer fresh".to_owned())
        ]
    );
    assert_eq!(refetch.asked_for(), vec!["old".to_owned()]);
}

#[tokio::test]
async fn a_second_401_is_returned_unchanged() {
    let refetch = ScriptedRefetch::new(vec![Some("fresh")]);
    let bearer = auth("old", Arc::clone(&refetch));
    let call = ScriptedCall::new(vec![401, 401]);
    assert_eq!(bearer.send_with_retry(&call).await.unwrap(), 401);
    assert_eq!(call.sent_tokens().len(), 2);
    assert_eq!(refetch.asked_for().len(), 1);
}

#[tokio::test]
async fn a_refetch_without_a_token_keeps_the_first_answer() {
    let refetch = ScriptedRefetch::new(vec![None]);
    let bearer = auth("old", Arc::clone(&refetch));
    let call = ScriptedCall::new(vec![401]);
    assert_eq!(bearer.send_with_retry(&call).await.unwrap(), 401);
    assert_eq!(call.sent_tokens(), vec![Some("Bearer old".to_owned())]);
    assert_eq!(refetch.asked_for(), vec!["old".to_owned()]);
}

#[tokio::test]
async fn a_fresh_token_that_cannot_be_a_header_value_is_misconfigured() {
    let refetch = ScriptedRefetch::new(vec![Some("bad\nvalue")]);
    let bearer = auth("old", refetch);
    let call = ScriptedCall::new(vec![401, 200]);
    let failure = bearer.send_with_retry(&call).await.unwrap_err();
    assert_eq!(
        failure,
        CredError::Misconfigured("invalid upstream header value for 'Authorization'".to_owned())
    );
    assert_eq!(call.sent_tokens().len(), 1);
}

#[test]
fn an_initial_token_that_cannot_be_a_header_value_is_misconfigured() {
    let refetch = ScriptedRefetch::new(Vec::new());
    let failure =
        ClientCredentialsBearerAuth::new(&secret("bad\nvalue"), refetch, HeaderCarrier::default())
            .unwrap_err();
    assert!(matches!(failure, CredError::Misconfigured(_)));
}

#[test]
fn debug_names_the_type_and_redacts_the_stored_token() {
    let refetch = ScriptedRefetch::new(Vec::new());
    let bearer = auth("secret-token", refetch);
    let rendered = format!("{bearer:?}");
    assert!(rendered.contains("ClientCredentialsBearerAuth"));
    assert!(!rendered.contains("secret-token"));
}

#[tokio::test]
async fn a_later_request_uses_the_replaced_token() {
    let refetch = ScriptedRefetch::new(vec![Some("fresh")]);
    let bearer = auth("old", refetch);
    let call = ScriptedCall::new(vec![401, 200]);
    assert_eq!(bearer.send_with_retry(&call).await.unwrap(), 200);
    let mut headers = HeaderMap::new();
    bearer.apply(&mut headers);
    assert_eq!(
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer fresh")
    );
}

#[tokio::test]
async fn the_carrier_names_the_header_and_formats_the_value() {
    let refetch = ScriptedRefetch::new(Vec::new());
    let carrier = HeaderCarrier::new("X-Api-Key", "").unwrap();
    let bearer = ClientCredentialsBearerAuth::new(&secret("tok"), refetch, carrier).unwrap();
    let mut headers = HeaderMap::new();
    bearer.apply(&mut headers);
    assert_eq!(
        headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok()),
        Some("tok")
    );
    assert!(headers.get("authorization").is_none());
}

struct GrantPost;

#[async_trait]
impl M2mTokenEndpointPost for GrantPost {
    async fn post(
        &self,
        _url: &str,
        _form: &[(String, SecretString)],
        _headers: &[(String, SecretString)],
    ) -> TokenEndpointOutcome {
        let body = serde_json::from_str("{\"access_token\":\"tok\"}").unwrap();
        TokenEndpointOutcome::Success { body }
    }
}

fn config() -> ClientCredentialsConfig {
    ClientCredentialsConfig {
        header: HeaderCarrier::default(),
        client_id: Some("client".to_owned()),
        client_secret: Some(secret("secret")),
        token_url: Some("https://idp.example.com/token".to_owned()),
        scopes: Vec::new(),
        audience: None,
        upstream_resource: None,
        token_endpoint_auth_method: None,
    }
}

fn source_refetch() -> SourceRefetch {
    let source = ClientCredentialsTokenSource::new(
        Arc::new(GrantPost),
        None,
        Arc::new(SystemWallClock),
        Tunables::default(),
    );
    SourceRefetch {
        source: Arc::new(source),
        server_id: "server".to_owned(),
        config: config(),
    }
}

#[tokio::test]
async fn source_refetch_grants_for_the_bound_server_and_config() {
    let fresh = source_refetch().refetch(&secret("stale")).await;
    assert_eq!(
        fresh.map(|value| value.expose_secret().to_owned()),
        Some("tok".to_owned())
    );
}

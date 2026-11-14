#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::sync::Mutex;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::{ExposeSecret as _, SecretString};

use super::{AgentAuth, auth_headers};
use crate::credential_sources::getters::VertexParams;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

fn vertex_params<'call>(
    project: Option<&'call str>,
    credentials: Option<&'call ConfiguredCredentials>,
) -> VertexParams<'call> {
    VertexParams {
        project,
        ai_project: None,
        location: None,
        ai_location: None,
        credentials,
        ai_credentials: None,
    }
}

fn text<'headers>(headers: &'headers BTreeMap<String, SecretString>, name: &str) -> &'headers str {
    headers
        .get(name)
        .map(SecretString::expose_secret)
        .unwrap_or_default()
}

fn token(value: &str) -> Result<AccessToken, CredentialCacheError> {
    Ok(AccessToken {
        token: SecretString::from(value),
        project: String::new(),
    })
}

struct RecordingMint {
    tokens: Vec<&'static str>,
    calls: Mutex<Vec<(Option<String>, Option<String>)>>,
}

impl RecordingMint {
    fn new(tokens: Vec<&'static str>) -> Self {
        Self {
            tokens,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(Option<String>, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

fn lookup<'mint>(
    mint: &'mint RecordingMint,
    credentials: Option<&ConfiguredCredentials>,
    project: Option<&str>,
) -> Ready<Result<AccessToken, CredentialCacheError>> {
    let mut calls = mint.calls.lock().unwrap();
    let index = calls.len();
    calls.push((
        credentials
            .and_then(ConfiguredCredentials::as_text)
            .map(str::to_owned),
        project.map(str::to_owned),
    ));
    ready(token(mint.tokens.get(index).copied().unwrap_or_default()))
}

#[tokio::test]
async fn caller_header_replacement() {
    let settings = VertexSettings::default();
    let auth = AgentAuth {
        params: vertex_params(Some("project"), None),
        vertex: &settings,
    };
    let caller = BTreeMap::from([
        ("Authorization".to_owned(), "Bearer caller".to_owned()),
        ("Content-Type".to_owned(), "text/plain".to_owned()),
        ("x-goog-user-project".to_owned(), "kept".to_owned()),
    ]);
    let headers = auth_headers(&auth, &caller, |_, _| ready(token("minted")))
        .await
        .unwrap();
    assert_eq!(text(&headers, "Authorization"), "Bearer minted");
    assert_eq!(text(&headers, "Content-Type"), "application/json");
    assert_eq!(text(&headers, "x-goog-user-project"), "kept");
}

#[tokio::test]
async fn identity_isolation() {
    let mint = RecordingMint::new(vec!["token-a", "token-b"]);
    let settings = VertexSettings::default();
    let credentials_a = ConfiguredCredentials::Text(SecretString::from("credential-a"));
    let credentials_b = ConfiguredCredentials::Text(SecretString::from("credential-b"));
    let first = AgentAuth {
        params: vertex_params(Some("project-a"), Some(&credentials_a)),
        vertex: &settings,
    };
    let second = AgentAuth {
        params: vertex_params(Some("project-b"), Some(&credentials_b)),
        vertex: &settings,
    };
    let headers_a = auth_headers(&first, &BTreeMap::new(), |credentials, project| {
        lookup(&mint, credentials, project)
    })
    .await
    .unwrap();
    let headers_b = auth_headers(&second, &BTreeMap::new(), |credentials, project| {
        lookup(&mint, credentials, project)
    })
    .await
    .unwrap();
    assert_eq!(text(&headers_a, "Authorization"), "Bearer token-a");
    assert_eq!(text(&headers_b, "Authorization"), "Bearer token-b");
    assert_eq!(
        mint.calls(),
        vec![
            (
                Some("credential-a".to_owned()),
                Some("project-a".to_owned())
            ),
            (
                Some("credential-b".to_owned()),
                Some("project-b".to_owned())
            ),
        ]
    );
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};

use super::{EnvironmentRequest, validate_environment};
use crate::vertex_files::error::VertexFilesError;

struct FixedMint {
    token: &'static str,
    calls: Mutex<u32>,
}

impl FixedMint {
    fn new(token: &'static str) -> Self {
        Self {
            token,
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }
}

impl MintAccessToken for FixedMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        _project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        let mut calls = self.calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        let token = SecretString::from(self.token);
        Box::pin(ready(Ok(AccessToken {
            token,
            project: "project-1".to_owned(),
        })))
    }
}

struct FailingMint {
    calls: Mutex<u32>,
}

impl MintAccessToken for FailingMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        _project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        let mut calls = self.calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        Box::pin(ready(Err(CredentialCacheError::MissingCredentials)))
    }
}

fn block_on<Call, Output>(future: Call) -> Output
where
    Call: Future<Output = Output>,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn caller_headers() -> BTreeMap<String, String> {
    BTreeMap::from([("x-test".to_owned(), "yes".to_owned())])
}

#[test]
fn an_explicit_key_leaves_the_headers_untouched() {
    let mint = FixedMint::new("minted");
    let headers = block_on(validate_environment(
        EnvironmentRequest {
            headers: caller_headers(),
            api_key: Some(&SecretString::from("explicit")),
            credentials: None,
            project: None,
        },
        &mint,
    ))
    .unwrap();
    assert_eq!(headers, caller_headers());
    assert_eq!(mint.calls(), 0);
}

#[test]
fn an_explicit_key_keeps_a_caller_authorization_header() {
    let mint = FixedMint::new("minted");
    let mut given = caller_headers();
    given.insert("Authorization".to_owned(), "Bearer caller".to_owned());
    let headers = block_on(validate_environment(
        EnvironmentRequest {
            headers: given.clone(),
            api_key: Some(&SecretString::from("explicit")),
            credentials: None,
            project: None,
        },
        &mint,
    ))
    .unwrap();
    assert_eq!(headers, given);
    assert_eq!(mint.calls(), 0);
}

#[test]
fn an_empty_key_counts_as_unset() {
    let mint = FixedMint::new("minted");
    let headers = block_on(validate_environment(
        EnvironmentRequest {
            headers: caller_headers(),
            api_key: Some(&SecretString::from("")),
            credentials: None,
            project: None,
        },
        &mint,
    ))
    .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer minted")
    );
    assert_eq!(mint.calls(), 1);
}

#[test]
fn a_missing_key_gets_the_minted_bearer_token() {
    let mint = FixedMint::new("minted");
    let mut headers = block_on(validate_environment(
        EnvironmentRequest {
            headers: caller_headers(),
            api_key: None,
            credentials: None,
            project: Some("project-2"),
        },
        &mint,
    ))
    .unwrap();
    assert_eq!(
        headers.remove("Authorization").as_deref(),
        Some("Bearer minted")
    );
    assert_eq!(headers, caller_headers());
    assert_eq!(mint.calls(), 1);
}

#[test]
fn an_empty_minted_token_is_refused() {
    let mint = FixedMint::new("");
    let failure = block_on(validate_environment(
        EnvironmentRequest {
            headers: caller_headers(),
            api_key: None,
            credentials: None,
            project: None,
        },
        &mint,
    ))
    .expect_err("an empty token is not a key");
    assert!(matches!(failure, VertexFilesError::MissingApiKey));
}

#[test]
fn a_failing_mint_is_reported() {
    let mint = FailingMint {
        calls: Mutex::new(0),
    };
    let failure = block_on(validate_environment(
        EnvironmentRequest {
            headers: caller_headers(),
            api_key: None,
            credentials: None,
            project: None,
        },
        &mint,
    ))
    .expect_err("the mint failed");
    assert!(matches!(failure, VertexFilesError::Credentials(_)));
    assert_eq!(*mint.calls.lock().unwrap(), 1);
}

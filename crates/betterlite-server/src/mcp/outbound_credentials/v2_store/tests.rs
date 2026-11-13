#![cfg(test)]

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Number;

use super::{CredentialPayload, CredentialReader, CredentialValue, V2PerUserTokenStore};
use crate::mcp::outbound_credentials::{OAuthToken, OAuthTokenStore as _, TokenStoreUnavailable};

struct ScriptedReader {
    results: Mutex<VecDeque<Result<Option<CredentialPayload>, TokenStoreUnavailable>>>,
    reads: AtomicUsize,
}

impl ScriptedReader {
    fn new(results: Vec<Result<Option<CredentialPayload>, TokenStoreUnavailable>>) -> Self {
        Self {
            results: Mutex::new(VecDeque::from(results)),
            reads: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl CredentialReader for ScriptedReader {
    async fn read_credential(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> Result<Option<CredentialPayload>, TokenStoreUnavailable> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted result per read")
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn text(value: &str) -> CredentialValue {
    CredentialValue::Text(value.to_owned())
}

fn payload() -> CredentialPayload {
    CredentialPayload::default()
}

fn token(access_token: &str) -> OAuthToken {
    OAuthToken {
        access_token: access_token.to_owned(),
        expires_at: None,
        refresh_token: None,
        scopes: Vec::new(),
        identity_binding_proof: None,
    }
}

fn store(
    results: Vec<Result<Option<CredentialPayload>, TokenStoreUnavailable>>,
) -> V2PerUserTokenStore<ScriptedReader> {
    V2PerUserTokenStore {
        reader: ScriptedReader::new(results),
    }
}

#[tokio::test]
async fn v2_store_parses_a_valid_payload() {
    let mut payload = payload();
    payload.access_token = text("access-secret");
    payload.refresh_token = text("refresh-secret");
    payload.expires_at = text("2024-01-01T00:00:00+00:00");
    payload.scopes = CredentialValue::Array(vec![text("read"), text("write")]);
    payload.identity_binding_proof = text("proof");
    let store = store(vec![Ok(Some(payload))]);
    let expected = OAuthToken {
        access_token: "access-secret".to_owned(),
        expires_at: Some(at(1_704_067_200)),
        refresh_token: Some("refresh-secret".to_owned()),
        scopes: vec!["read".to_owned(), "write".to_owned()],
        identity_binding_proof: Some("proof".to_owned()),
    };
    assert_eq!(store.fetch("user", "server").await, Ok(Some(expected)));
}

#[tokio::test]
async fn v2_store_defaults_the_optional_fields() {
    let mut payload = payload();
    payload.access_token = text("access-secret");
    let store = store(vec![Ok(Some(payload))]);
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("access-secret")))
    );
}

#[tokio::test]
async fn v2_store_returns_none_without_an_access_token() {
    let store = store(vec![Ok(Some(payload()))]);
    assert_eq!(store.fetch("user", "server").await, Ok(None));
}

#[tokio::test]
async fn v2_store_returns_none_for_a_non_string_access_token() {
    let mut payload = payload();
    payload.access_token = CredentialValue::Number(Number::from(7_i64));
    let store = store(vec![Ok(Some(payload))]);
    assert_eq!(store.fetch("user", "server").await, Ok(None));
}

#[tokio::test]
async fn v2_store_returns_none_when_no_credential_is_stored() {
    let store = store(vec![Ok(None)]);
    assert_eq!(store.fetch("user", "server").await, Ok(None));
}

#[tokio::test]
async fn v2_store_propagates_an_unavailable_reader() {
    let unavailable = TokenStoreUnavailable {
        detail: "Database not connected".to_owned(),
    };
    let store = store(vec![Err(unavailable.clone())]);
    assert_eq!(store.fetch("user", "server").await, Err(unavailable));
}

#[tokio::test]
async fn v2_store_returns_none_for_an_empty_user_id_without_reading() {
    let store = store(vec![Ok(Some(payload()))]);
    assert_eq!(store.fetch("", "server").await, Ok(None));
    assert_eq!(store.reader.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn v2_store_filters_non_string_scopes() {
    let mut payload = payload();
    payload.access_token = text("access-secret");
    payload.scopes = CredentialValue::Array(vec![
        text("read"),
        CredentialValue::Number(Number::from(1_i64)),
        text("write"),
        CredentialValue::Array(vec![text("nested")]),
    ]);
    let store = store(vec![Ok(Some(payload))]);
    let mut expected = token("access-secret");
    expected.scopes = vec!["read".to_owned(), "write".to_owned()];
    assert_eq!(store.fetch("user", "server").await, Ok(Some(expected)));
}

#[tokio::test]
async fn v2_store_returns_empty_scopes_for_a_non_list() {
    let mut payload = payload();
    payload.access_token = text("access-secret");
    payload.scopes = text("read write");
    let store = store(vec![Ok(Some(payload))]);
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("access-secret")))
    );
}

#[tokio::test]
async fn v2_store_parses_a_naive_iso_expiry_as_utc() {
    let mut payload = payload();
    payload.access_token = text("access-secret");
    payload.expires_at = text("2024-01-01T00:00:00");
    let store = store(vec![Ok(Some(payload))]);
    let mut expected = token("access-secret");
    expected.expires_at = Some(at(1_704_067_200));
    assert_eq!(store.fetch("user", "server").await, Ok(Some(expected)));
}

#[tokio::test]
async fn v2_store_drops_a_malformed_expiry() {
    let mut payload = payload();
    payload.access_token = text("access-secret");
    payload.expires_at = text("not-a-date");
    let store = store(vec![Ok(Some(payload))]);
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("access-secret")))
    );
}

#[tokio::test]
async fn v2_store_ignores_a_non_string_expiry() {
    let mut payload = payload();
    payload.access_token = text("access-secret");
    payload.expires_at = CredentialValue::Number(Number::from(1_704_067_200_i64));
    let store = store(vec![Ok(Some(payload))]);
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("access-secret")))
    );
}

#[test]
fn v2_store_decodes_payload_json_with_mixed_field_types() {
    let payload: CredentialPayload = serde_json::from_str(
        r#"{"access_token":"access","refresh_token":7,"expires_at":null,"scopes":["read",2],"type":"oauth2"}"#,
    )
    .unwrap();
    assert_eq!(payload.access_token, text("access"));
    assert_eq!(
        payload.refresh_token,
        CredentialValue::Number(Number::from(7_i64))
    );
    assert_eq!(payload.expires_at, CredentialValue::Null);
    assert_eq!(
        payload.scopes,
        CredentialValue::Array(vec![
            text("read"),
            CredentialValue::Number(Number::from(2_i64))
        ])
    );
    assert_eq!(payload.identity_binding_proof, CredentialValue::Null);
}

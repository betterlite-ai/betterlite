use std::collections::BTreeMap;

use async_trait::async_trait;
use aws_lc_rs::hmac;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_key_lookup::{KeyAuthenticator, KeyLookupOutcome};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::Serialize;

use crate::byok_consent::get::GetQuery;
use crate::byok_consent::store::AuthCodeStore;
use crate::byok_token_exchange::ports::AuthCodeRecord;
use crate::registry::servers::{RegistrySettings, ServerRegistry};
use crate::server_record::MCPServer;

pub(crate) const MASTER_KEY: &str = "sk-oracle-master";
pub(crate) const NOW: i64 = 1_700_000_000;
pub(crate) const FUTURE_EXPIRY: i64 = 4_102_444_800;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct Claims<'text> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) iat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) login_method: Option<&'text str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) user_id: Option<&'text str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) key: Option<&'text str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub(crate) token_type: Option<&'text str>,
}

pub(crate) fn ui_claims() -> Claims<'static> {
    Claims {
        exp: Some(FUTURE_EXPIRY),
        iat: None,
        login_method: Some("sso"),
        user_id: Some("user-1"),
        key: None,
        token_type: None,
    }
}

pub(crate) fn token(claims: Claims<'_>, master_key: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_string(&claims).unwrap());
    let signing_input = format!("{header}.{payload}");
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, master_key.as_bytes()),
        signing_input.as_bytes(),
    );
    format!(
        "token={signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    )
}

pub(crate) fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(NOW, 0).unwrap()
}

pub(crate) fn master(value: &str) -> SecretString {
    SecretString::from(value)
}

pub(crate) fn record(expires_at: f64) -> AuthCodeRecord {
    AuthCodeRecord {
        api_key: SecretString::from("sk-seed"),
        server_id: String::new(),
        code_challenge: String::new(),
        redirect_uri: String::new(),
        client_id: String::new(),
        user_id: String::new(),
        expires_at,
    }
}

pub(crate) fn seeded_store(size: usize, expires_at: f64) -> AuthCodeStore {
    let codes = AuthCodeStore::new();
    for index in 0..size {
        codes.insert(format!("seed-{index}"), record(expires_at));
    }
    codes
}

pub(crate) fn page_query() -> GetQuery {
    GetQuery {
        client_id: Some("client-1".to_owned()),
        redirect_uri: Some("http://127.0.0.1:1456/callback".to_owned()),
        response_type: Some("code".to_owned()),
        code_challenge: Some("challenge".to_owned()),
        code_challenge_method: None,
        state: Some("state-1".to_owned()),
        server_id: None,
    }
}

pub(crate) fn registry(servers: Vec<MCPServer>) -> ServerRegistry {
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: false,
            internal_ip_ranges: None,
        },
        servers,
        Vec::new(),
    )
}

pub(crate) fn server(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

pub(crate) struct Keys {
    pub(crate) resolves: bool,
}

#[async_trait]
impl KeyAuthenticator for Keys {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError> {
        if !self.resolves {
            return Err(KeyLookupError::MissingKey {
                key: presented.to_owned(),
            });
        }
        let auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap();
        Ok(KeyLookupOutcome {
            auth,
            hashed_token: presented.to_owned(),
        })
    }
}

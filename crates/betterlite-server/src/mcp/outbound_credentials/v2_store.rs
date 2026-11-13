mod tests;

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Number;

use super::{OAuthToken, OAuthTokenStore, TokenStoreUnavailable, iso_to_epoch};

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CredentialValue {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct CredentialPayload {
    #[serde(default)]
    pub access_token: CredentialValue,
    #[serde(default)]
    pub refresh_token: CredentialValue,
    #[serde(default)]
    pub expires_at: CredentialValue,
    #[serde(default)]
    pub scopes: CredentialValue,
    #[serde(default)]
    pub identity_binding_proof: CredentialValue,
}

fn text_of(value: &CredentialValue) -> Option<&str> {
    if let CredentialValue::Text(text) = value {
        Some(text)
    } else {
        None
    }
}

pub fn to_oauth_token(payload: &CredentialPayload) -> Option<OAuthToken> {
    let access_token = text_of(&payload.access_token)?;
    let scopes = if let CredentialValue::Array(items) = &payload.scopes {
        items
            .iter()
            .filter_map(|item| text_of(item).map(str::to_owned))
            .collect()
    } else {
        Vec::new()
    };
    Some(OAuthToken {
        access_token: access_token.to_owned(),
        expires_at: text_of(&payload.expires_at).and_then(iso_to_epoch),
        refresh_token: text_of(&payload.refresh_token).map(str::to_owned),
        scopes,
        identity_binding_proof: text_of(&payload.identity_binding_proof).map(str::to_owned),
    })
}

#[async_trait]
pub trait CredentialReader: Send + Sync {
    async fn read_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<CredentialPayload>, TokenStoreUnavailable>;
}

pub struct V2PerUserTokenStore<Reader> {
    pub reader: Reader,
}

#[async_trait]
impl<Reader: CredentialReader> OAuthTokenStore for V2PerUserTokenStore<Reader> {
    async fn fetch(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        if user_id.is_empty() {
            return Ok(None);
        }
        let payload = self.reader.read_credential(user_id, server_id).await?;
        Ok(payload.as_ref().and_then(to_oauth_token))
    }
}

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_user_credentials::rows::CredentialRows;
use betterlite_db::mcp_user_credentials::store::get_user_oauth_credential;
use betterlite_db::mcp_user_credentials::store_user_oauth_credential;
use betterlite_db::mcp_user_credentials::types::{
    CredentialError, OAuthCredentialPayload, StoreOAuthCredential,
};
use betterlite_db::oauth_token_resolution::OauthBindingPort;
use betterlite_db::oauth_token_resolution::ports::TokenResponse;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_mcp::oauth_identity_binding::port_types::{BindingDenied, RefreshOwnership};
use betterlite_mcp::server_record::MCPServer;
use betterlite_mcp::server_record::binding::BindingMode;
use http::Method;
use http::StatusCode;
use http::header::{
    ACCEPT, HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

use super::super::authz_code_refresher::ports::{
    CredentialPersist, PersistRequest, ServerLookup, TokenEndpointPost,
};
use super::super::v2_store::{CredentialPayload, CredentialReader, CredentialValue};
use super::super::{TokenStoreUnavailable, WallClock};

const DATABASE_NOT_CONNECTED: &str = "Database not connected";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
const JSON_ACCEPT: &str = "application/json";

pub struct PooledTokenPost {
    pub client: Arc<ProxiedClient>,
    pub deadline: Duration,
}

impl TokenEndpointPost for PooledTokenPost {
    async fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> Option<TokenResponse> {
        let request = match refresh_request(url, form, headers, self.deadline) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!("MCP OAuth refresh request failed: {error}");
                return None;
            }
        };
        let sent = match self.client.send(request).await {
            Ok(sent) => sent,
            Err(error) => {
                tracing::warn!("MCP OAuth refresh request failed: {error}");
                return None;
            }
        };
        let status = sent.response.status();
        let buffered = match sent.buffered(self.client.limits.body_bytes).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!("MCP OAuth refresh request failed: {error}");
                return None;
            }
        };
        answer(status, &buffered.body)
    }
}

#[derive(Debug, Error)]
pub(super) enum RefreshRequestError {
    #[error("the token URL is not a URL")]
    Url,
    #[error(transparent)]
    HeaderName(#[from] InvalidHeaderName),
    #[error(transparent)]
    HeaderValue(#[from] InvalidHeaderValue),
}

// The Accept default comes first and a given Accept replaces it, as LiteLLM's {"Accept": ..., **headers} dict does.
pub(super) fn refresh_request(
    url: &str,
    form: &BTreeMap<String, String>,
    headers: &BTreeMap<String, String>,
    deadline: Duration,
) -> Result<OutboundRequest, RefreshRequestError> {
    let parsed = Url::parse(url).map_err(|_error| RefreshRequestError::Url)?;
    let mut header_map = HeaderMap::new();
    header_map.insert(ACCEPT, HeaderValue::from_static(JSON_ACCEPT));
    for (name, value) in headers {
        let header_name = HeaderName::try_from(name.as_str())?;
        let mut header_value = HeaderValue::try_from(value.as_str())?;
        header_value.set_sensitive(true);
        header_map.insert(header_name, header_value);
    }
    let data = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            form.iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .finish()
        .into_bytes();
    Ok(OutboundRequest {
        method: Method::POST,
        url: parsed,
        query: Vec::new(),
        headers: header_map,
        body: RequestBody::Bytes {
            content_type: HeaderValue::from_static(FORM_CONTENT_TYPE),
            data,
        },
        deadline,
    })
}

// A failed refresh is a miss, never a 500: the status and the body both end in None with the reason logged.
fn answer(status: StatusCode, body: &[u8]) -> Option<TokenResponse> {
    if !status.is_success() {
        let status_code = status.as_u16();
        tracing::warn!(
            "MCP OAuth refresh request failed: the token endpoint answered HTTP {status_code}"
        );
        return None;
    }
    match serde_json::from_slice::<JsonValue>(body) {
        Ok(JsonValue::Object(object)) => Some(object),
        Ok(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_),
        ) => {
            tracing::warn!(
                "MCP OAuth refresh request failed: the token endpoint body is not a JSON object"
            );
            None
        }
        Err(error) => {
            tracing::warn!("MCP OAuth refresh request failed: {error}");
            None
        }
    }
}

pub struct DbCredentialReader<Table> {
    pub database: Option<Table>,
    pub cipher: Arc<DbCipher>,
}

#[async_trait]
impl<Table: CredentialRows> CredentialReader for DbCredentialReader<Table> {
    async fn read_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<CredentialPayload>, TokenStoreUnavailable> {
        match &self.database {
            None => Err(TokenStoreUnavailable {
                detail: DATABASE_NOT_CONNECTED.to_owned(),
            }),
            Some(table) => get_user_oauth_credential(table, &self.cipher, user_id, server_id)
                .await
                .map_err(|error| unavailable(&error))
                .map(|payload| payload.map(credential_payload)),
        }
    }
}

pub struct DbCredentialPersist<Table> {
    pub database: Option<Table>,
    pub cipher: Arc<DbCipher>,
    pub clock: Arc<dyn WallClock>,
}

impl<Table: CredentialRows> CredentialPersist for DbCredentialPersist<Table> {
    async fn persist(&self, request: &PersistRequest<'_>) -> Result<(), TokenStoreUnavailable> {
        let Some(table) = &self.database else {
            return Ok(());
        };
        let access_token = SecretString::from(request.access_token.to_owned());
        let refresh_token = request
            .refresh_token
            .map(|token| SecretString::from(token.to_owned()));
        let stored = StoreOAuthCredential {
            user_id: request.user_id,
            server_id: request.server_id,
            access_token: &access_token,
            refresh_token: refresh_token.as_ref(),
            expires_in: request.expires_in,
            scopes: request.scopes,
            skip_byok_guard: true,
            identity_binding_proof: request.identity_binding_proof,
        };
        store_user_oauth_credential(table, &self.cipher, &stored, self.clock.now())
            .await
            .map_err(|error| unavailable(&error))
    }
}

pub struct BoundCredentialReader<Lookup, Binding, Reader> {
    pub server_lookup: Arc<Lookup>,
    pub binding: Arc<Binding>,
    pub reader: Arc<Reader>,
}

#[async_trait]
impl<Lookup, Binding, Reader> CredentialReader for BoundCredentialReader<Lookup, Binding, Reader>
where
    Lookup: ServerLookup,
    Binding:
        OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>,
    Reader: CredentialReader,
{
    async fn read_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<CredentialPayload>, TokenStoreUnavailable> {
        let Some(credential) = self.reader.read_credential(user_id, server_id).await? else {
            return Ok(None);
        };
        let Some(server) = self.server_lookup.server(server_id) else {
            return Ok(Some(credential));
        };
        let Some(binding) = &server.fields.oauth_identity_binding else {
            return Ok(Some(credential));
        };
        if binding.mode != BindingMode::Enforce {
            return Ok(Some(credential));
        }
        if self
            .binding
            .proof_matches(&server, user_id, credential_proof(&credential))
            .await
        {
            return Ok(Some(credential));
        }
        Ok(None)
    }
}

fn credential_proof(credential: &CredentialPayload) -> Option<&str> {
    match &credential.identity_binding_proof {
        CredentialValue::Text(text) => Some(text),
        CredentialValue::Null
        | CredentialValue::Bool(_)
        | CredentialValue::Number(_)
        | CredentialValue::Array(_)
        | CredentialValue::Object(_) => None,
    }
}

fn unavailable(error: &CredentialError) -> TokenStoreUnavailable {
    TokenStoreUnavailable {
        detail: error.to_string(),
    }
}

fn credential_payload(payload: OAuthCredentialPayload) -> CredentialPayload {
    CredentialPayload {
        access_token: secret_value(payload.access_token.as_ref()),
        refresh_token: secret_value(payload.refresh_token.as_ref()),
        expires_at: text_value(payload.expires_at.as_deref()),
        scopes: scopes_value(payload.scopes),
        identity_binding_proof: text_value(payload.identity_binding_proof.as_deref()),
    }
}

fn secret_value(value: Option<&SecretString>) -> CredentialValue {
    value.map_or(CredentialValue::Null, |text| {
        CredentialValue::Text(text.expose_secret().to_owned())
    })
}

fn text_value(value: Option<&str>) -> CredentialValue {
    value.map_or(CredentialValue::Null, |text| {
        CredentialValue::Text(text.to_owned())
    })
}

fn scopes_value(scopes: Option<Vec<String>>) -> CredentialValue {
    scopes.map_or(CredentialValue::Null, |items| {
        CredentialValue::Array(items.into_iter().map(CredentialValue::Text).collect())
    })
}

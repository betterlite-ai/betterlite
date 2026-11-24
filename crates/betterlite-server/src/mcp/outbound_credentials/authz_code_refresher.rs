pub mod grant_body;
pub mod ports;
mod tests;

use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_db::mcp_stored_contracts::MCPTokenEndpointAuthMethod;
use betterlite_db::oauth_token_resolution::ports::TokenResponse;
use betterlite_db::oauth_token_resolution::{EnforceRequest, OauthBindingPort};
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointClientAuth;
use betterlite_mcp::oauth_identity_binding::port_types::{BindingDenied, RefreshOwnership};
use betterlite_mcp::oauth_resource::resource_identity::{
    UpstreamServer, build_upstream_token_request,
};
use betterlite_mcp::server_record::MCPServer;
use betterlite_mcp::server_record::binding::BindingMode;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use grant_body::{GrantBody, read_grant_body};
use ports::{CredentialPersist, EpochClock, PersistRequest, ServerLookup, TokenEndpointPost};

use super::{OAuthToken, TokenRefresher, TokenStoreUnavailable};

pub struct AuthorizationCodeRefresher<Lookup, Post, Persist, Binding, Clock> {
    pub server_lookup: Lookup,
    pub token_endpoint: Post,
    pub persist: Persist,
    pub binding: Binding,
    pub clock: Clock,
}

// The user, server and presented refresh token that one refresh carries through its steps.
struct RefreshCall<'call> {
    user_id: &'call str,
    server_id: &'call str,
    refresh_token: &'call str,
}

#[async_trait]
impl<Lookup, Post, Persist, Binding, Clock> TokenRefresher
    for AuthorizationCodeRefresher<Lookup, Post, Persist, Binding, Clock>
where
    Lookup: ServerLookup,
    Post: TokenEndpointPost,
    Persist: CredentialPersist,
    Binding:
        OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>,
    Clock: EpochClock,
{
    async fn refresh(
        &self,
        user_id: &str,
        server_id: &str,
        token: &OAuthToken,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        let Some(refresh_token) = token.refresh_token.as_deref() else {
            return Ok(None);
        };
        let Some(server) = self.server_lookup.server(server_id) else {
            return Ok(None);
        };
        let Some(token_url) = server.effective_token_url().filter(|url| !url.is_empty()) else {
            return Ok(None);
        };
        let call = RefreshCall {
            user_id,
            server_id,
            refresh_token,
        };
        let Some((grant, proof)) = self.grant_for(&call, &server, token_url).await else {
            return Ok(None);
        };
        let (new_refresh, scopes) = self
            .persist_grant(&call, &grant, token, proof.as_deref())
            .await?;
        Ok(Some(OAuthToken {
            access_token: grant.access_token,
            expires_at: grant
                .expires_in
                .and_then(|expires_in| expiry_from(self.clock.now(), expires_in)),
            refresh_token: new_refresh,
            scopes,
            identity_binding_proof: proof,
        }))
    }
}

impl<Lookup, Post, Persist, Binding, Clock>
    AuthorizationCodeRefresher<Lookup, Post, Persist, Binding, Clock>
where
    Lookup: ServerLookup,
    Post: TokenEndpointPost,
    Persist: CredentialPersist,
    Binding:
        OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>,
    Clock: EpochClock,
{
    fn client_auth(server: &MCPServer, server_id: &str) -> Option<TokenEndpointClientAuth> {
        let upstream = UpstreamServer {
            upstream_resource: server.fields.upstream_resource.as_deref(),
            url: server.fields.url.as_deref(),
            server_id: server.fields.server_id.as_str(),
        };
        let auth_method = server
            .fields
            .token_endpoint_auth_method
            .map(auth_method_name);
        let client_secret = server
            .fields
            .client_secret
            .as_ref()
            .map(|credential| &credential.0.0);
        match build_upstream_token_request(
            &upstream,
            auth_method,
            server.fields.client_id.as_deref(),
            client_secret,
        ) {
            Ok(client_auth) => Some(client_auth),
            Err(error) => {
                tracing::warn!("MCP OAuth refresh misconfigured for server {server_id}: {error}");
                None
            }
        }
    }

    // The pre-check runs only for an enforced binding, before any network call, with an empty token response.
    async fn precheck_binding(&self, server: &MCPServer, call: &RefreshCall<'_>) -> bool {
        let Some(binding) = &server.fields.oauth_identity_binding else {
            return true;
        };
        if binding.mode != BindingMode::Enforce {
            return true;
        }
        let empty = TokenResponse::new();
        self.enforce(server, call, &empty).await.is_ok()
    }

    // The second check runs for every server; the port answers Ok(None) for an unbound or audit one.
    async fn enforce(
        &self,
        server: &MCPServer,
        call: &RefreshCall<'_>,
        token_response: &TokenResponse,
    ) -> Result<Option<String>, BindingDenied> {
        let ownership =
            RefreshOwnership::Presented(SecretString::from(call.refresh_token.to_owned()));
        let request = EnforceRequest {
            server,
            token_response,
            user_id: Some(call.user_id),
            grant_type: "refresh_token",
            ownership: Some(&ownership),
        };
        self.binding.enforce(&request).await
    }

    // The pre-check, the POST and the second check in order, each failure reading as no grant.
    async fn grant_for(
        &self,
        call: &RefreshCall<'_>,
        server: &MCPServer,
        token_url: &str,
    ) -> Option<(GrantBody, Option<String>)> {
        let client_auth = Self::client_auth(server, call.server_id)?;
        if !self.precheck_binding(server, call).await {
            return None;
        }
        let form = refresh_form(call.refresh_token, &client_auth.body);
        let headers = exposed(&client_auth.headers);
        let body = self
            .token_endpoint
            .post_form(token_url, &form, &headers)
            .await?;
        let grant = read_grant_body(&body)?;
        let proof = self.enforce(server, call, &body).await.ok()?;
        Some((grant, proof))
    }

    // Writes the grant and answers the refresh token and scopes the returned token carries.
    async fn persist_grant(
        &self,
        call: &RefreshCall<'_>,
        grant: &GrantBody,
        token: &OAuthToken,
        proof: Option<&str>,
    ) -> Result<(Option<String>, Vec<String>), TokenStoreUnavailable> {
        let refresh_token = grant
            .rotated_refresh
            .as_deref()
            .or(token.refresh_token.as_deref());
        // LiteLLM's "or token.scopes": an all-whitespace scope string parses to an empty tuple, which is falsy.
        let scopes = grant
            .scopes
            .clone()
            .filter(|scopes| !scopes.is_empty())
            .unwrap_or_else(|| token.scopes.clone());
        let request = PersistRequest {
            user_id: call.user_id,
            server_id: call.server_id,
            access_token: &grant.access_token,
            refresh_token,
            expires_in: grant.expires_in,
            scopes: (!scopes.is_empty()).then_some(scopes.as_slice()),
            identity_binding_proof: proof,
        };
        self.persist.persist(&request).await?;
        Ok((refresh_token.map(str::to_owned), scopes))
    }
}

const fn auth_method_name(method: MCPTokenEndpointAuthMethod) -> &'static str {
    match method {
        MCPTokenEndpointAuthMethod::ClientSecretBasic => "client_secret_basic",
        MCPTokenEndpointAuthMethod::ClientSecretPost => "client_secret_post",
    }
}

fn refresh_form(
    refresh_token: &str,
    body: &BTreeMap<String, SecretString>,
) -> BTreeMap<String, String> {
    let mut form = BTreeMap::from([
        ("grant_type".to_owned(), "refresh_token".to_owned()),
        ("refresh_token".to_owned(), refresh_token.to_owned()),
    ]);
    form.extend(exposed(body));
    form
}

fn exposed(values: &BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(field, value)| (field.clone(), value.expose_secret().to_owned()))
        .collect()
}

// The instant LiteLLM gets from clock() + expires_in, kept as milliseconds so no float operator is used.
fn expiry_from(now_epoch_seconds: f64, expires_in: i64) -> Option<DateTime<Utc>> {
    let seconds = num_traits::cast::<i64, f64>(expires_in)?;
    let epoch_seconds = now_epoch_seconds.mul_add(1.0_f64, seconds);
    let epoch_millis = num_traits::cast::<f64, i64>(epoch_seconds.mul_add(1_000.0_f64, 0.0_f64))?;
    DateTime::from_timestamp_millis(epoch_millis)
}

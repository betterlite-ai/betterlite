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

use grant_body::read_grant_body;
use ports::{
    CredentialPersist, EpochClock, PersistRequest, ServerLookup, SystemEpochClock,
    TokenEndpointPost,
};

use super::{OAuthToken, TokenRefresher, TokenStoreUnavailable};

pub struct AuthorizationCodeRefresher<Lookup, Post, Persist, Binding, Clock> {
    pub server_lookup: Lookup,
    pub token_endpoint: Post,
    pub persist: Persist,
    pub binding: Binding,
    pub clock: Clock,
}

impl<Lookup, Post, Persist, Binding>
    AuthorizationCodeRefresher<Lookup, Post, Persist, Binding, SystemEpochClock>
{
    #[must_use]
    pub fn new(
        server_lookup: Lookup,
        token_endpoint: Post,
        persist: Persist,
        binding: Binding,
    ) -> Self {
        Self {
            server_lookup,
            token_endpoint,
            persist,
            binding,
            clock: SystemEpochClock,
        }
    }
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
        let Some(client_auth) = self.client_auth(&server, server_id) else {
            return Ok(None);
        };
        if !self.precheck_binding(&server, user_id, refresh_token).await {
            return Ok(None);
        }
        let form = refresh_form(refresh_token, &client_auth.body);
        let headers = exposed(&client_auth.headers);
        let Some(body) = self
            .token_endpoint
            .post_form(token_url, &form, &headers)
            .await
        else {
            return Ok(None);
        };
        let Some(grant) = read_grant_body(&body) else {
            return Ok(None);
        };
        let Ok(proof) = self.enforce(&server, user_id, refresh_token, &body).await else {
            return Ok(None);
        };
        let new_refresh = grant
            .rotated_refresh
            .unwrap_or_else(|| refresh_token.to_owned());
        let scopes = grant.scopes.unwrap_or_else(|| token.scopes.clone());
        {
            let request = PersistRequest {
                user_id,
                server_id,
                access_token: &grant.access_token,
                refresh_token: Some(&new_refresh),
                expires_in: grant.expires_in,
                scopes: (!scopes.is_empty()).then_some(scopes.as_slice()),
                identity_binding_proof: proof.as_deref(),
            };
            self.persist.persist(&request).await?;
        }
        Ok(Some(OAuthToken {
            access_token: grant.access_token,
            expires_at: grant
                .expires_in
                .and_then(|expires_in| expiry_from(self.clock.now(), expires_in)),
            refresh_token: Some(new_refresh),
            scopes,
            identity_binding_proof: proof,
        }))
    }
}

impl<Lookup, Post, Persist, Binding, Clock>
    AuthorizationCodeRefresher<Lookup, Post, Persist, Binding, Clock>
where
    Binding:
        OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>,
{
    fn client_auth(&self, server: &MCPServer, server_id: &str) -> Option<TokenEndpointClientAuth> {
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
    async fn precheck_binding(
        &self,
        server: &MCPServer,
        user_id: &str,
        refresh_token: &str,
    ) -> bool {
        let Some(binding) = &server.fields.oauth_identity_binding else {
            return true;
        };
        if binding.mode != BindingMode::Enforce {
            return true;
        }
        let empty = TokenResponse::new();
        self.enforce(server, user_id, refresh_token, &empty)
            .await
            .is_ok()
    }

    // The second check runs for every server; the port answers Ok(None) for an unbound or audit one.
    async fn enforce(
        &self,
        server: &MCPServer,
        user_id: &str,
        refresh_token: &str,
        token_response: &TokenResponse,
    ) -> Result<Option<String>, BindingDenied> {
        let ownership = RefreshOwnership::Presented(SecretString::from(refresh_token.to_owned()));
        let request = EnforceRequest {
            server,
            token_response,
            user_id: Some(user_id),
            grant_type: "refresh_token",
            ownership: Some(&ownership),
        };
        self.binding.enforce(&request).await
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

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use betterlite_mcp::credential_contracts::auth_config::id_jag::ClientAuth;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use betterlite_mcp::outbound_credentials::collaborators::{
    BoxFuture, ExchangeRequest, ExchangedToken, OAuthToken, TokenEndpointClient, TokenExchanger,
};

use super::inspect::client_auth_answer;

const NO_REPLY: &str = "no canned token endpoint reply";
pub(super) const NOT_WIRED: &str = "token exchange collaborator not wired";
const NO_TOKEN: &str = "the recording exchanger has no token";

pub(super) struct RecordingEndpoint {
    replies: Mutex<VecDeque<ExchangedToken>>,
    calls: Mutex<Vec<Value>>,
}

impl RecordingEndpoint {
    pub(super) fn new(access_tokens: &[String]) -> Self {
        let replies = access_tokens
            .iter()
            .map(|access_token| ExchangedToken {
                access_token: SecretString::from(access_token.clone()),
                expires_in: None,
            })
            .collect();
        Self {
            replies: Mutex::new(replies),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TokenEndpointClient for RecordingEndpoint {
    fn fetch<'call>(
        &'call self,
        endpoint: &'call str,
        client_id: &'call str,
        grant_params: &'call BTreeMap<String, String>,
        client_auth: &'call ClientAuth,
    ) -> BoxFuture<'call, CredResult<ExchangedToken>> {
        let record = json!({
            "endpoint": endpoint,
            "client_id": client_id,
            "params": grant_params,
            "client_auth": client_auth_answer(client_auth),
        });
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        Box::pin(
            async move { reply.ok_or_else(|| CredError::UpstreamUnavailable(NO_REPLY.to_owned())) },
        )
    }
}

pub(super) struct RecordingExchanger {
    token: Option<OAuthToken>,
    error: Option<CredError>,
    exchanges: Mutex<Vec<Value>>,
    invalidations: Mutex<Vec<Value>>,
}

impl RecordingExchanger {
    pub(super) fn returning(access_token: &str) -> Self {
        Self {
            token: Some(oauth_token(access_token)),
            error: None,
            exchanges: Mutex::new(Vec::new()),
            invalidations: Mutex::new(Vec::new()),
        }
    }

    pub(super) const fn failing(error: CredError) -> Self {
        Self {
            token: None,
            error: Some(error),
            exchanges: Mutex::new(Vec::new()),
            invalidations: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn exchanges(&self) -> Vec<Value> {
        let guard = self
            .exchanges
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }

    pub(super) fn invalidations(&self) -> Vec<Value> {
        let guard = self
            .invalidations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

impl TokenExchanger for RecordingExchanger {
    fn exchange<'call>(
        &'call self,
        request: ExchangeRequest<'call>,
    ) -> BoxFuture<'call, CredResult<OAuthToken>> {
        record_exchange(&self.exchanges, request.subject_token, request.tenant_id);
        let reply = match (&self.error, &self.token) {
            (Some(error), _token) => Err(error.clone()),
            (None, Some(token)) => Ok(token.clone()),
            (None, None) => Err(CredError::Misconfigured(NO_TOKEN.to_owned())),
        };
        Box::pin(async move { reply })
    }

    fn invalidate<'call>(&'call self, request: ExchangeRequest<'call>) -> BoxFuture<'call, ()> {
        record_exchange(
            &self.invalidations,
            request.subject_token,
            request.tenant_id,
        );
        Box::pin(async {})
    }
}

fn record_exchange(records: &Mutex<Vec<Value>>, subject_token: &SecretString, tenant_id: &str) {
    let record = json!({"subject_token": subject_token.expose_secret(), "tenant_id": tenant_id});
    records
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(record);
}

pub(super) fn oauth_token(access_token: &str) -> OAuthToken {
    OAuthToken {
        access_token: SecretString::from(access_token.to_owned()),
        expires_at: None,
        scopes: Vec::new(),
    }
}

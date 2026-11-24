use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::oauth_token_resolution::ports::TokenResponse;
use betterlite_db::oauth_token_resolution::{EnforceRequest, OauthBindingPort};
use betterlite_mcp::oauth_identity_binding::port_types::{
    BindingDenied, RefreshOwnership, RejectionCode,
};
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::outbound_credentials::authz_code_refresher::ports::{
    CredentialPersist, EpochClock, PersistRequest, ServerLookup, TokenEndpointPost,
};
use betterlite_server::mcp::outbound_credentials::{OAuthToken, TokenStoreUnavailable};
use chrono::DateTime;
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

pub(super) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error("RuntimeError", error.to_string()))
}

pub(super) fn server_record(value: &Value) -> Result<MCPServer, Box<HelperError>> {
    serde_json::from_value(value.clone())
        .map_err(|error| helper_error("InvalidInput", error.to_string()))
}

// The case's answer is plain JSON, so it becomes the token response the way the HTTP adapter reads a body.
pub(super) fn token_response(value: &Value) -> Result<TokenResponse, Box<HelperError>> {
    serde_json::from_str(&value.to_string())
        .map_err(|error| helper_error("InvalidInput", error.to_string()))
}

#[derive(Debug, Deserialize)]
pub(super) struct TokenInput {
    pub(super) access_token: String,
    #[serde(default)]
    pub(super) refresh_token: Option<String>,
    #[serde(default)]
    pub(super) scopes: Vec<String>,
    #[serde(default)]
    pub(super) identity_binding_proof: Option<String>,
}

impl From<&TokenInput> for OAuthToken {
    fn from(input: &TokenInput) -> Self {
        Self {
            access_token: input.access_token.clone(),
            expires_at: None,
            refresh_token: input.refresh_token.clone(),
            scopes: input.scopes.clone(),
            identity_binding_proof: input.identity_binding_proof.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct TokenOutput {
    access_token: String,
    expires_at_millis: Option<i64>,
    refresh_token: Option<String>,
    scopes: Vec<String>,
    identity_binding_proof: Option<String>,
}

impl From<OAuthToken> for TokenOutput {
    fn from(token: OAuthToken) -> Self {
        let OAuthToken {
            access_token,
            expires_at,
            refresh_token,
            scopes,
            identity_binding_proof,
        } = token;
        Self {
            access_token,
            expires_at_millis: expires_at.as_ref().map(DateTime::timestamp_millis),
            refresh_token,
            scopes,
            identity_binding_proof,
        }
    }
}

pub(super) struct FixedLookup(pub(super) Option<Arc<MCPServer>>);

impl ServerLookup for FixedLookup {
    fn server(&self, _server_id: &str) -> Option<Arc<MCPServer>> {
        self.0.clone()
    }
}

pub(super) struct FixedClock(pub(super) f64);

impl EpochClock for FixedClock {
    fn now(&self) -> f64 {
        self.0
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct PostCall {
    url: String,
    form: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
}

#[derive(Default)]
pub(super) struct ScriptedPost {
    pub(super) answer: Option<TokenResponse>,
    pub(super) calls: Mutex<Vec<PostCall>>,
}

impl TokenEndpointPost for ScriptedPost {
    fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> impl Future<Output = Option<TokenResponse>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(PostCall {
                url: url.to_owned(),
                form: form.clone(),
                headers: headers.clone(),
            });
        ready(self.answer.clone())
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct PersistCall {
    user_id: String,
    server_id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scopes: Option<Vec<String>>,
    identity_binding_proof: Option<String>,
}

#[derive(Default)]
pub(super) struct RecordingPersist {
    pub(super) calls: Mutex<Vec<PersistCall>>,
}

impl CredentialPersist for RecordingPersist {
    fn persist(
        &self,
        request: &PersistRequest<'_>,
    ) -> impl Future<Output = Result<(), TokenStoreUnavailable>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(PersistCall {
                user_id: request.user_id.to_owned(),
                server_id: request.server_id.to_owned(),
                access_token: request.access_token.to_owned(),
                refresh_token: request.refresh_token.map(str::to_owned),
                expires_in: request.expires_in,
                scopes: request.scopes.map(<[String]>::to_vec),
                identity_binding_proof: request.identity_binding_proof.map(str::to_owned),
            });
        ready(Ok(()))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(super) enum ValidatorAnswer {
    Proof { proof: String },
    None,
    Denied,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ValidatorCall {
    user_id: Option<String>,
    grant_type: String,
    token_response: TokenResponse,
    refresh_token: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ProofCall {
    user_id: String,
    server_id: String,
    proof: Option<String>,
}

pub(super) struct ScriptedBinding {
    answers: Mutex<VecDeque<ValidatorAnswer>>,
    matches: Mutex<VecDeque<bool>>,
    pub(super) validator_calls: Mutex<Vec<ValidatorCall>>,
    pub(super) proof_calls: Mutex<Vec<ProofCall>>,
}

impl ScriptedBinding {
    pub(super) fn new(answers: Vec<ValidatorAnswer>, matches: Vec<bool>) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            matches: Mutex::new(VecDeque::from(matches)),
            validator_calls: Mutex::new(Vec::new()),
            proof_calls: Mutex::new(Vec::new()),
        }
    }
}

impl OauthBindingPort for ScriptedBinding {
    type Server = MCPServer;
    type Ownership = RefreshOwnership;
    type Denied = BindingDenied;

    fn enforce(
        &self,
        request: &EnforceRequest<'_, MCPServer, RefreshOwnership>,
    ) -> impl Future<Output = Result<Option<String>, BindingDenied>> + Send {
        let refresh_token = match request.ownership {
            Some(RefreshOwnership::Presented(token)) => Some(token.expose_secret().to_owned()),
            Some(RefreshOwnership::Proven) | None => None,
        };
        self.validator_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ValidatorCall {
                user_id: request.user_id.map(str::to_owned),
                grant_type: request.grant_type.to_owned(),
                token_response: request.token_response.clone(),
                refresh_token,
            });
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(ValidatorAnswer::None);
        ready(match answer {
            ValidatorAnswer::Proof { proof } => Ok(Some(proof)),
            ValidatorAnswer::None => Ok(None),
            ValidatorAnswer::Denied => Err(BindingDenied {
                code: RejectionCode::BindingFailed,
                description: "the scripted validator denied the refresh".to_owned(),
                server_id: request.server.fields.server_id.clone(),
            }),
        })
    }

    fn proof_matches(
        &self,
        server: &MCPServer,
        user_id: &str,
        stored_proof: Option<&str>,
    ) -> impl Future<Output = bool> + Send {
        self.proof_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ProofCall {
                user_id: user_id.to_owned(),
                server_id: server.fields.server_id.clone(),
                proof: stored_proof.map(str::to_owned),
            });
        ready(
            self.matches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or(false),
        )
    }
}

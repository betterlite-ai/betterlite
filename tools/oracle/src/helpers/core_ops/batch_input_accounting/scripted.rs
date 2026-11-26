use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::batch_input_accounting::error::{BatchHttpError, BatchManagedFileError};
use betterlite_core::api::batch_input_accounting::ports::{
    BatchAccessCheck, BatchFileFetch, BatchFileRequest, BatchFileSource, BatchKeyGrant,
    BatchModelAccess, BatchTeamLookup,
};
use betterlite_core::api::batch_limit_counters::rejections::PROJECT_OTPM_DESCRIPTOR_KEY;
use betterlite_core::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use betterlite_core::api::batch_limit_descriptors::limiter::BatchLimiterSource;
use betterlite_core::api::batch_output_usage::tokens::BatchTokenCounter;
use betterlite_core::api::model_credentials::CredentialSource;
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_rate_limit::token_estimation::output_cap::{output_candidate_count, output_floor};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::TokenRequest;
use serde_json::{Value, json};

use super::input::{AccessScript, ContentScript, Hook, Lookup, ManagedScript, Outcome};

pub(super) struct ScriptedFiles {
    content: ContentScript,
    managed: ManagedScript,
    pub(super) fetched: Mutex<Vec<Value>>,
}

pub(super) struct ScriptedAccess {
    script: AccessScript,
}

pub(super) struct ScriptedCounter {
    answers: Mutex<VecDeque<Option<i64>>>,
    pub(super) counted: Mutex<Vec<Value>>,
}

// The team the fake lookup returns: the models the team is allowed to call
pub(super) struct TeamModels(pub(super) Vec<String>);

pub(super) struct LiteLlmLimiter;

pub(super) struct NoCredentials;

impl ContentScript {
    fn outcome(&self) -> BatchFileFetch {
        if let Some(http) = &self.http {
            return BatchFileFetch::Http(BatchHttpError {
                status: http.status,
                detail: http.detail.clone().unwrap_or(Json::Null),
            });
        }
        if let Some(kind) = &self.not_bytes {
            return BatchFileFetch::NotBytes { kind: kind.clone() };
        }
        let bytes = self.contents.as_deref().unwrap_or("").as_bytes().to_vec();
        BatchFileFetch::Content(bytes)
    }
}

impl ManagedScript {
    fn unavailable(&self) -> Option<BatchManagedFileError> {
        if self.logging == Some(false) {
            return Some(BatchManagedFileError::LoggingMissing);
        }
        match self.hook.unwrap_or(Hook::Endpoints) {
            Hook::Missing => return Some(BatchManagedFileError::HookMissing),
            Hook::Plain => return Some(BatchManagedFileError::NotFileEndpoints),
            Hook::Endpoints => {}
        }
        if self.router == Some(false) {
            return Some(BatchManagedFileError::RouterMissing);
        }
        None
    }
}

impl ScriptedFiles {
    pub(super) const fn new(content: ContentScript, managed: ManagedScript) -> Self {
        Self {
            content,
            managed,
            fetched: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, request: BatchFileRequest<'_>) {
        let entry = match request {
            BatchFileRequest::Managed { file_id, .. } => {
                json!({"kind": "managed", "file_id": file_id, "caller": true})
            }
            BatchFileRequest::Provider {
                file_id,
                params,
                caller,
            } => json!({
                "kind": "provider",
                "file_id": file_id,
                "params": params,
                "caller": caller.is_some(),
            }),
        };
        self.fetched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    fn outcome(&self, managed: bool) -> BatchFileFetch {
        if managed {
            if let Some(reason) = self.managed.unavailable() {
                return BatchFileFetch::ManagedUnavailable(reason);
            }
        }
        self.content.outcome()
    }
}

#[async_trait]
impl BatchFileSource for ScriptedFiles {
    type Error = Infallible;

    async fn fetch(&self, request: BatchFileRequest<'_>) -> Result<BatchFileFetch, Infallible> {
        let managed = matches!(request, BatchFileRequest::Managed { .. });
        let outcome = self.outcome(managed);
        // A managed read whose dependency is missing never reaches the backing store
        if !matches!(outcome, BatchFileFetch::ManagedUnavailable(_)) {
            self.record(request);
        }
        Ok(outcome)
    }
}

fn http_denial() -> BatchHttpError {
    let mut detail = JsonObject::new();
    detail.insert("error".to_owned(), Json::Text("passed through".to_owned()));
    BatchHttpError {
        status: 401,
        detail: Json::Object(detail),
    }
}

impl ScriptedAccess {
    pub(super) const fn new(script: AccessScript) -> Self {
        Self { script }
    }

    fn listed(&self, model: &str, allowed: &[String]) -> BatchAccessCheck {
        if allowed.is_empty() || allowed.iter().any(|name| name == "*" || name == model) {
            BatchAccessCheck::Allowed
        } else {
            BatchAccessCheck::Denied {
                message: self.script.message.clone(),
            }
        }
    }

    fn key_check(&self) -> BatchAccessCheck {
        match self.script.key {
            Outcome::Allow => BatchAccessCheck::Allowed,
            Outcome::Deny => BatchAccessCheck::Denied {
                message: self.script.message.clone(),
            },
            Outcome::Fail => BatchAccessCheck::Failed {
                message: self.script.message.clone(),
            },
            Outcome::Http => BatchAccessCheck::Http(http_denial()),
        }
    }
}

#[async_trait]
impl BatchModelAccess for ScriptedAccess {
    type Team = TeamModels;
    type Error = Infallible;

    fn database_available(&self) -> bool {
        self.script.database
    }

    async fn team(&self, _team_id: &str) -> Result<BatchTeamLookup<TeamModels>, Infallible> {
        Ok(match self.script.team {
            Lookup::Loaded => BatchTeamLookup::Loaded(TeamModels(self.script.team_models.clone())),
            Lookup::Failed => BatchTeamLookup::Failed,
            Lookup::Http => BatchTeamLookup::Http(http_denial()),
        })
    }

    async fn team_can_access(
        &self,
        model: &str,
        _caller: &UserAPIKeyAuth,
        team: &TeamModels,
    ) -> Result<BatchAccessCheck, Infallible> {
        Ok(self.listed(model, &team.0))
    }

    async fn key_group_grants(
        &self,
        _model: &str,
        _caller: &UserAPIKeyAuth,
        _team: &TeamModels,
    ) -> Result<BatchKeyGrant, Infallible> {
        Ok(BatchKeyGrant::Denied)
    }

    async fn member_can_access(
        &self,
        _model: &str,
        _caller: &UserAPIKeyAuth,
        _team: &TeamModels,
    ) -> Result<BatchAccessCheck, Infallible> {
        Ok(BatchAccessCheck::Allowed)
    }

    async fn key_can_call(
        &self,
        _model: &str,
        _caller: &UserAPIKeyAuth,
    ) -> Result<BatchAccessCheck, Infallible> {
        Ok(self.key_check())
    }
}

fn counted_shape(request: &TokenRequest<'_>) -> Value {
    if let Some(text) = request.text {
        return json!({"kind": "text", "model": request.model, "text": text.joined()});
    }
    match request.messages {
        Some(messages) => json!({
            "kind": "messages",
            "model": request.model,
            "roles": messages.iter().map(|message| message.role.as_str()).collect::<Vec<_>>(),
        }),
        None => json!({"kind": "none", "model": request.model}),
    }
}

impl ScriptedCounter {
    pub(super) fn new(answers: Vec<Option<i64>>) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            counted: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl BatchTokenCounter for ScriptedCounter {
    async fn count_tokens(&self, request: &TokenRequest<'_>) -> Result<usize, CountError> {
        self.counted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(counted_shape(request));
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match answer {
            Some(Some(tokens)) => Ok(usize::try_from(tokens).unwrap_or(0)),
            Some(None) => Err(CountError::MissingInput),
            None => Ok(0),
        }
    }
}

impl BatchLimiterSource for LiteLlmLimiter {
    type Error = Infallible;

    const PROJECT_OTPM_DESCRIPTOR_KEY: &'static str = PROJECT_OTPM_DESCRIPTOR_KEY;

    fn descriptors_for(
        &self,
        _auth: &UserAPIKeyAuth,
        _data: &RequestData,
    ) -> Result<Vec<BatchLimitDescriptor>, Infallible> {
        Ok(Vec::new())
    }

    fn project_io_for(
        &self,
        _auth: &UserAPIKeyAuth,
        _model: &str,
        _out: &mut Vec<BatchLimitDescriptor>,
    ) {
    }

    fn candidate_count(&self, body: &RequestData) -> i64 {
        output_candidate_count(body, None)
    }

    fn output_floor(&self, min_configured_otpm_limit: Option<i64>) -> i64 {
        output_floor(min_configured_otpm_limit, None)
    }
}

impl CredentialSource for NoCredentials {
    type Error = Infallible;

    fn credentials_with_provider(
        &self,
        _model_id: &str,
        _team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, Infallible> {
        Ok(None)
    }
}

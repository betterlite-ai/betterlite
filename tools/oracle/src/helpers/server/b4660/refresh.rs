use std::sync::{Arc, Mutex, PoisonError};

use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::mcp::outbound_credentials::authz_code_refresher::AuthorizationCodeRefresher;
use betterlite_server::mcp::outbound_credentials::per_user_oauth_store::chain::SharedBinding;
use betterlite_server::mcp::outbound_credentials::{OAuthToken, TokenRefresher as _};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{
    FixedClock, FixedLookup, RecordingPersist, ScriptedBinding, ScriptedPost, TokenInput,
    TokenOutput, ValidatorAnswer, helper_error, runtime, server_record, token_response,
};

#[derive(Debug, Deserialize)]
pub(crate) struct RefreshInput {
    pub(super) server_id: String,
    pub(super) user_id: String,
    pub(super) server: Option<Value>,
    pub(super) token: TokenInput,
    #[serde(default)]
    pub(super) post_answer: Option<Value>,
    #[serde(default)]
    pub(super) validators: Vec<ValidatorAnswer>,
    pub(super) now: f64,
}

pub(super) fn refresh(input: &RefreshInput) -> HelperResult {
    let server = match &input.server {
        Some(value) => Some(Arc::new(server_record(value)?)),
        None => None,
    };
    let binding = Arc::new(ScriptedBinding::new(input.validators.clone(), Vec::new()));
    let answer = input.post_answer.as_ref().map(token_response).transpose()?;
    let post = Arc::new(ScriptedPost {
        answer,
        calls: Mutex::new(Vec::new()),
    });
    let persist = Arc::new(RecordingPersist::default());
    let refresher = AuthorizationCodeRefresher {
        server_lookup: Arc::new(FixedLookup(server)),
        token_endpoint: Arc::clone(&post),
        persist: Arc::clone(&persist),
        binding: SharedBinding(Arc::clone(&binding)),
        clock: FixedClock(input.now),
    };
    let token = OAuthToken::from(&input.token);
    let outcome = runtime()?
        .block_on(refresher.refresh(&input.user_id, &input.server_id, &token))
        .map_err(|error| helper_error("TokenStoreUnavailable", error.detail))?;
    Ok(json!({
        "token": outcome.map(TokenOutput::from),
        "posts": post.calls.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        "persists": persist.calls.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        "validators": binding
            .validator_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    }))
}

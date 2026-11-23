use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::mcp::outbound_credentials::per_user_oauth_store::chain::{
    BuiltStore, StoreBuilder,
};
use betterlite_server::mcp::outbound_credentials::per_user_oauth_store::lazy::{
    LazyPerUserOAuthTokenStore, RedisAvailability,
};
use betterlite_server::mcp::outbound_credentials::{
    InvalidatableOAuthTokenStore, OAuthToken, OAuthTokenStore, TokenStoreUnavailable,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::support::{
    FixedLookup, ScriptedBinding, TokenInput, TokenOutput, helper_error, runtime, server_record,
};

#[derive(Debug, Deserialize)]
pub(crate) struct LazyInput {
    pub(super) server: Option<Value>,
    pub(super) token: Option<TokenInput>,
    #[serde(default)]
    pub(super) builds: Vec<bool>,
    #[serde(default)]
    pub(super) binding_matches: Vec<bool>,
    pub(super) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Step {
    Fetch { user_id: String, server_id: String },
    Invalidate { user_id: String, server_id: String },
    SetRedisAvailable { available: bool },
}

#[derive(Debug, Serialize)]
pub(super) struct StepOutput {
    op: String,
    token: Option<TokenOutput>,
}

struct ChainStore {
    token: Option<OAuthToken>,
    fetches: AtomicUsize,
    invalidations: Mutex<Vec<String>>,
}

#[async_trait]
impl OAuthTokenStore for ChainStore {
    async fn fetch(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        self.fetches.fetch_add(1, Ordering::Relaxed);
        Ok(self.token.clone())
    }
}

#[async_trait]
impl InvalidatableOAuthTokenStore for ChainStore {
    async fn invalidate(&self, _user_id: &str, server_id: &str) {
        self.invalidations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(server_id.to_owned());
    }
}

struct ScriptedBuilder {
    store: Arc<ChainStore>,
    answers: Mutex<VecDeque<bool>>,
    builds: Mutex<Vec<bool>>,
}

impl StoreBuilder for ScriptedBuilder {
    fn build(&self) -> BuiltStore {
        let uses_redis = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(false);
        self.builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(uses_redis);
        let store: Arc<dyn InvalidatableOAuthTokenStore> = Arc::clone(&self.store);
        BuiltStore { store, uses_redis }
    }
}

struct ScriptedAvailability(AtomicBool);

impl RedisAvailability for ScriptedAvailability {
    fn is_available(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub(super) fn lazy_fetch(input: &LazyInput) -> HelperResult {
    let server = match &input.server {
        Some(value) => Some(Arc::new(server_record(value)?)),
        None => None,
    };
    let binding = Arc::new(ScriptedBinding::new(
        Vec::new(),
        input.binding_matches.clone(),
    ));
    let chain = Arc::new(ChainStore {
        token: input.token.as_ref().map(OAuthToken::from),
        fetches: AtomicUsize::new(0),
        invalidations: Mutex::new(Vec::new()),
    });
    let availability = Arc::new(ScriptedAvailability(AtomicBool::new(false)));
    let store = LazyPerUserOAuthTokenStore::new(
        Arc::new(FixedLookup(server)),
        Arc::clone(&binding),
        ScriptedBuilder {
            store: Arc::clone(&chain),
            answers: Mutex::new(VecDeque::from(input.builds.clone())),
            builds: Mutex::new(Vec::new()),
        },
        Arc::clone(&availability),
    );
    let steps = runtime()?.block_on(async {
        let mut outputs = Vec::new();
        for step in &input.steps {
            let (op, token) = match step {
                Step::Fetch { user_id, server_id } => (
                    "fetch",
                    store
                        .fetch(user_id, server_id)
                        .await
                        .map_err(|error| helper_error("TokenStoreUnavailable", error.detail))?,
                ),
                Step::Invalidate { user_id, server_id } => {
                    store.invalidate(user_id, server_id).await;
                    ("invalidate", None)
                }
                Step::SetRedisAvailable { available } => {
                    availability.0.store(*available, Ordering::Relaxed);
                    ("set_redis_available", None)
                }
            };
            outputs.push(StepOutput {
                op: op.to_owned(),
                token: token.map(TokenOutput::from),
            });
        }
        Ok::<_, Box<HelperError>>(outputs)
    })?;
    Ok(json!({
        "steps": steps,
        "builds": store.store_builder.builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        "store_fetches": chain.fetches.load(Ordering::Relaxed),
        "store_invalidations": chain
            .invalidations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        "binding_checks": binding
            .proof_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    }))
}

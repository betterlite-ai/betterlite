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
    builds: Arc<Mutex<Vec<bool>>>,
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
        let chain: Arc<ChainStore> = Arc::clone(&self.store);
        let store: Arc<dyn InvalidatableOAuthTokenStore> = chain;
        BuiltStore { store, uses_redis }
    }
}

struct ScriptedAvailability(AtomicBool);

impl RedisAvailability for ScriptedAvailability {
    fn is_available(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

struct LazyParts {
    store: LazyPerUserOAuthTokenStore<
        FixedLookup,
        ScriptedBinding,
        ScriptedBuilder,
        ScriptedAvailability,
    >,
    chain: Arc<ChainStore>,
    binding: Arc<ScriptedBinding>,
    availability: Arc<ScriptedAvailability>,
    builds: Arc<Mutex<Vec<bool>>>,
}

fn build_parts(input: &LazyInput) -> Result<LazyParts, Box<HelperError>> {
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
    let builds = Arc::new(Mutex::new(Vec::new()));
    let store = LazyPerUserOAuthTokenStore::new(
        Arc::new(FixedLookup(server)),
        Arc::clone(&binding),
        ScriptedBuilder {
            store: Arc::clone(&chain),
            answers: Mutex::new(VecDeque::from(input.builds.clone())),
            builds: Arc::clone(&builds),
        },
        Arc::clone(&availability),
    );
    Ok(LazyParts {
        store,
        chain,
        binding,
        availability,
        builds,
    })
}

async fn run_steps(parts: &LazyParts, steps: &[Step]) -> Result<Vec<StepOutput>, Box<HelperError>> {
    let mut outputs = Vec::new();
    for step in steps {
        let (op, token) = match step {
            Step::Fetch { user_id, server_id } => (
                "fetch",
                parts
                    .store
                    .fetch(user_id, server_id)
                    .await
                    .map_err(|error| helper_error("TokenStoreUnavailable", error.detail))?,
            ),
            Step::Invalidate { user_id, server_id } => {
                parts.store.invalidate(user_id, server_id).await;
                ("invalidate", None)
            }
            Step::SetRedisAvailable { available } => {
                parts.availability.0.store(*available, Ordering::Relaxed);
                ("set_redis_available", None)
            }
        };
        outputs.push(StepOutput {
            op: op.to_owned(),
            token: token.map(TokenOutput::from),
        });
    }
    Ok(outputs)
}

pub(super) fn lazy_fetch(input: &LazyInput) -> HelperResult {
    let parts = build_parts(input)?;
    let steps = runtime()?.block_on(run_steps(&parts, &input.steps))?;
    Ok(json!({
        "steps": steps,
        "builds": parts
            .builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        "store_fetches": parts.chain.fetches.load(Ordering::Relaxed),
        "store_invalidations": parts
            .chain
            .invalidations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        "binding_checks": parts
            .binding
            .proof_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    }))
}

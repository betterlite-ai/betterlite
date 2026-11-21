use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::cache_cost_routing as section;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::{Content, Message};
use rust_decimal::Decimal;
use serde_json::Number;
use tokio::time::sleep;

use crate::cache_cost_routing::{
    CacheAwareChoice, CacheCostRequest, CacheCostRouting, CachedModelChooser, ChooseCached,
    PreRoutingResponse, TierParams,
};
use crate::complexity_scoring::decision::RoutingDecision;
use crate::complexity_scoring::tier::ComplexityTier;
use crate::context_placement::facts::WindowFacts;
use crate::context_placement::fit::ContextFit;
use crate::router_predicates::JsonMap;

pub(super) const TIMEOUT_MS: u64 = 2000;

pub(super) fn config() -> section::Config {
    section::Config {
        enabled: true,
        output_tokens: 1024,
        timeout_ms: TIMEOUT_MS,
    }
}

pub(super) fn message() -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text("hello".to_owned())),
        name: None,
        provider_specific_fields: JsonMap::new(),
    }
}

pub(super) fn response() -> PreRoutingResponse {
    PreRoutingResponse {
        model: "small".to_owned(),
        messages: Some(vec![message()]),
        routing_decision: Some(decision()),
        session_affinity_ttl_seconds: Some(3600),
        litellm_params: Some(params(&[("max_tokens", 512)])),
    }
}

fn decision() -> RoutingDecision {
    [
        ("router_model_name", Json::Text("probe".to_owned())),
        ("router_type", Json::Text("complexity".to_owned())),
        ("routed_model", Json::Text("small".to_owned())),
        ("cause", Json::Text("heuristic_scorer".to_owned())),
        ("tier", Json::Text("SIMPLE".to_owned())),
        ("conversation_continuing", Json::Bool(true)),
        (
            "signals",
            Json::List(vec![Json::Text("short (2 tokens)".to_owned())]),
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

pub(super) fn params(entries: &[(&str, u64)]) -> JsonMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Json::Number(Number::from(*value))))
        .collect()
}

pub(super) fn fit(facts: &[(&str, Option<u64>)], needed: Option<u64>) -> ContextFit {
    ContextFit {
        facts: facts
            .iter()
            .map(|(model, window)| {
                (
                    (*model).to_owned(),
                    WindowFacts {
                        window: *window,
                        has_unknown: window.is_none(),
                    },
                )
            })
            .collect(),
        needed,
        buffer: 0.8_f64,
    }
}

pub(super) fn choice(estimated: &str, original: &str) -> CacheAwareChoice {
    CacheAwareChoice {
        model: "medium".to_owned(),
        tier: "MEDIUM".to_owned(),
        deployment_id: "dep-medium".to_owned(),
        original_cost: original.parse::<Decimal>().unwrap(),
        estimated_cost: estimated.parse::<Decimal>().unwrap(),
    }
}

pub(super) struct FixedParams {
    pub(super) params: JsonMap,
    calls: AtomicUsize,
}

impl FixedParams {
    pub(super) fn new(params: JsonMap) -> Self {
        Self {
            params,
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl TierParams for FixedParams {
    fn params_for_model(&self, _tier: &str, _model: &str) -> JsonMap {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.params.clone()
    }
}

#[derive(Clone)]
pub(super) struct Seen {
    pub(super) model: String,
    pub(super) decision_present: bool,
    pub(super) output_tokens: u64,
    pub(super) messages_present: bool,
    pub(super) kwargs_keys: Vec<String>,
}

pub(super) struct FixedChooser {
    choice: Option<CacheAwareChoice>,
    delay: Duration,
    calls: AtomicUsize,
    seen: Mutex<Option<Seen>>,
}

impl FixedChooser {
    pub(super) fn new(choice: Option<CacheAwareChoice>, delay: Duration) -> Self {
        Self {
            choice,
            delay,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(None),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(super) fn seen(&self) -> Option<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CachedModelChooser for FixedChooser {
    async fn choose(&self, request: ChooseCached<'_>) -> Option<CacheAwareChoice> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let seen = Seen {
            model: request.response.model.clone(),
            decision_present: request.response.routing_decision.is_some(),
            output_tokens: request.config.output_tokens,
            messages_present: request.messages.is_some(),
            kwargs_keys: request.request_kwargs.keys().cloned().collect(),
        };
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(seen);
        sleep(self.delay).await;
        self.choice.clone()
    }
}

pub(super) struct DropGuard(pub(super) Arc<AtomicBool>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub(super) struct CancelledChooser {
    pub(super) dropped: Arc<AtomicBool>,
}

#[async_trait]
impl CachedModelChooser for CancelledChooser {
    async fn choose(&self, _request: ChooseCached<'_>) -> Option<CacheAwareChoice> {
        let _guard = DropGuard(Arc::clone(&self.dropped));
        sleep(Duration::from_secs(30)).await;
        None
    }
}

pub(super) struct Fixture {
    pub(super) config: section::Config,
    pub(super) labels: BTreeMap<ComplexityTier, String>,
    pub(super) params: FixedParams,
    pub(super) fit: ContextFit,
    pub(super) kwargs: JsonMap,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            config: config(),
            labels: BTreeMap::new(),
            params: FixedParams::new(params(&[("max_tokens", 8192)])),
            fit: fit(&[], None),
            kwargs: JsonMap::new(),
        }
    }

    pub(super) async fn apply(
        &self,
        chooser: &dyn CachedModelChooser,
        response: Option<PreRoutingResponse>,
        messages: Option<&[Message]>,
    ) -> Option<PreRoutingResponse> {
        let service = CacheCostRouting {
            config: &self.config,
            tier_labels: &self.labels,
            params_for_model: &self.params,
            chooser,
        };
        service
            .apply(CacheCostRequest {
                response,
                messages,
                request_kwargs: &self.kwargs,
                context_fit: &self.fit,
            })
            .await
    }
}

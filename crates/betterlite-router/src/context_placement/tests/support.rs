use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::context_placement::Config;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};
use indexmap::IndexMap;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::complexity_scoring::tier::Tier;
use crate::context_placement::ContextPlacement;
use crate::context_placement::error::PlacementError;
use crate::context_placement::facts::{ModelLimitKey, RouterModelInfo, WindowFacts};
use crate::context_placement::placement::PlacementRequest;
use crate::context_placement::request::{OutOfBand, RequestCounter, RequestFacts};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::tier_definitions::model::{TierModelEntry, TierPool, TierSet};

pub(super) fn config(enabled: bool, buffer: f64) -> Config {
    Config { enabled, buffer }
}

pub(super) fn message(content: &str) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: Some(MessageContent::Text(content.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

pub(super) fn json(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn request_facts<'a>(
    messages: Option<&'a [ChatMessage]>,
    out_of_band: OutOfBand<'a>,
    compaction_pending: bool,
) -> RequestFacts<'a> {
    RequestFacts {
        messages,
        out_of_band,
        compaction_pending,
    }
}

pub(super) fn placement_request<'a>(
    tier: &'a Tier,
    set: &'a TierSet,
    request: RequestFacts<'a>,
) -> PlacementRequest<'a> {
    PlacementRequest {
        tier,
        set,
        default_model: None,
        pool_override: None,
        request,
    }
}

pub(super) fn known_window(window: u64) -> WindowFacts {
    WindowFacts {
        window: Some(window),
        has_unknown: false,
    }
}

pub(super) fn deployment(
    model_name: &str,
    litellm_params: &str,
    model_info: &str,
) -> DeploymentSpec {
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: serde_json::from_str(litellm_params).unwrap(),
        model_info: serde_json::from_str(model_info).unwrap(),
        extra: IndexMap::new(),
    }
}

// the router's model map answer for the deployment, as get_router_model_info would return it
pub(super) fn with_router(mut spec: DeploymentSpec, info: &str) -> DeploymentSpec {
    spec.extra.insert("router_info".to_owned(), json(info));
    spec
}

pub(super) fn catalog(specs: Vec<DeploymentSpec>) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for spec in specs {
        catalog.push(spec);
    }
    catalog
}

pub(super) fn model_catalog(raw: &str) -> ModelCatalog {
    ModelCatalog::parse(raw.as_bytes(), UnknownFields::Reject).unwrap()
}

pub(super) fn tier_set(pools: &[(&str, &[&str])]) -> TierSet {
    TierSet {
        definitions: None,
        pools: pools
            .iter()
            .map(|(name, models)| {
                (
                    (*name).to_owned(),
                    TierPool::Pool(
                        models
                            .iter()
                            .map(|model| TierModelEntry {
                                model_name: (*model).to_owned(),
                                params: IndexMap::new(),
                            })
                            .collect(),
                    ),
                )
            })
            .collect(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

pub(super) fn service<'deps>(
    config: &'deps Config,
    deployments: &'deps DeploymentCatalog,
    model_info: &'deps dyn RouterModelInfo,
    counter: &'deps dyn RequestCounter,
) -> ContextPlacement<'deps> {
    ContextPlacement {
        config,
        deployments,
        model_info,
        counter,
    }
}

// the router's model map answer the test wrote into a deployment's extra fields
pub(super) struct ExtraModelInfo;

impl RouterModelInfo for ExtraModelInfo {
    fn limit(&self, _group: &str, deployment: &DeploymentSpec, key: ModelLimitKey) -> Option<u64> {
        let Some(Json::Object(info)) = deployment.extra.get("router_info") else {
            return None;
        };
        match info.get(key.field()) {
            Some(Json::Number(number)) => number.as_u64(),
            _ => None,
        }
    }
}

pub(super) struct FixedCounter {
    pub(super) value: u64,
}

#[async_trait]
impl RequestCounter for FixedCounter {
    async fn count(&self, _messages: &[ChatMessage], _text: &str) -> Result<u64, PlacementError> {
        Ok(self.value)
    }
}

pub(super) struct FailingCounter;

#[async_trait]
impl RequestCounter for FailingCounter {
    async fn count(&self, _messages: &[ChatMessage], _text: &str) -> Result<u64, PlacementError> {
        Err(PlacementError::Count {
            message: "the counter failed".to_owned(),
        })
    }
}

pub(super) struct RecordingCounter {
    seen: Mutex<Vec<(usize, String)>>,
}

impl RecordingCounter {
    pub(super) fn new() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<(usize, String)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RequestCounter for RecordingCounter {
    async fn count(&self, messages: &[ChatMessage], text: &str) -> Result<u64, PlacementError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((messages.len(), text.to_owned()));
        Ok(1)
    }
}

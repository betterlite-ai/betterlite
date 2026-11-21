use std::collections::BTreeMap;

use betterlite_prompts::message_content::Message;
use betterlite_router::complexity_scoring::decision::RoutingDecision;
use betterlite_router::router_predicates::JsonMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    #[serde(rename = "cache_aware_routing")]
    pub(super) enabled: bool,
    #[serde(rename = "cache_aware_routing_output_tokens")]
    pub(super) output_tokens: u64,
    #[serde(rename = "cache_aware_routing_timeout_ms")]
    pub(super) timeout_ms: u64,
    #[serde(default)]
    pub(super) tier_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) tiers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) params: Vec<ParamsInput>,
    #[serde(default)]
    pub(super) response: Option<ResponseInput>,
    #[serde(default)]
    pub(super) messages: Option<Vec<Message>>,
    #[serde(default)]
    pub(super) request_kwargs: JsonMap,
    pub(super) context_fit: FitInput,
    #[serde(default)]
    pub(super) choice: Option<ChoiceInput>,
    #[serde(default)]
    pub(super) chooser_delay_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParamsInput {
    pub(super) tier: String,
    pub(super) model: String,
    #[serde(default)]
    pub(super) params: JsonMap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResponseInput {
    pub(super) model: String,
    #[serde(default)]
    pub(super) routing_decision: Option<RoutingDecision>,
    #[serde(default)]
    pub(super) session_affinity_ttl_seconds: Option<u32>,
    #[serde(default)]
    pub(super) litellm_params: Option<JsonMap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FitInput {
    #[serde(default)]
    pub(super) facts: BTreeMap<String, FitFactsInput>,
    #[serde(default)]
    pub(super) needed: Option<u64>,
    pub(super) buffer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FitFactsInput {
    #[serde(default)]
    pub(super) window: Option<u64>,
    #[serde(default)]
    pub(super) has_unknown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChoiceInput {
    pub(super) model: String,
    pub(super) tier: String,
    pub(super) deployment_id: String,
    pub(super) original_cost: String,
    pub(super) estimated_cost: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CallOutput {
    pub(super) model: String,
    pub(super) decision_present: bool,
    pub(super) output_tokens: u64,
    pub(super) messages_present: bool,
    pub(super) kwargs_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ResponseOutput {
    pub(super) model: String,
    pub(super) messages_present: bool,
    pub(super) routing_decision: Option<RoutingDecision>,
    pub(super) session_affinity_ttl_seconds: Option<u32>,
    pub(super) litellm_params: Option<JsonMap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Output {
    pub(super) calls: u64,
    pub(super) call: Option<CallOutput>,
    pub(super) response: Option<ResponseOutput>,
}

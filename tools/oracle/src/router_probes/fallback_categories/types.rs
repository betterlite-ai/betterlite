use serde::{Deserialize, Serialize};
use serde_json::Value;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(pub Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub failure: Failure,
    #[serde(default)]
    pub disable_fallbacks: bool,
    #[serde(default)]
    pub guardrail_intervention: bool,
    pub original_model_group: Option<String>,
    pub model_group: String,
    #[serde(default)]
    pub pre_routing_selected: Option<String>,
    #[serde(default)]
    pub router_fallbacks: Vec<String>,
    pub ordinary: Ordinary,
    pub context_window: Chain,
    pub content_policy: Chain,
    pub order: Order,
    pub weighted_failover: Weighted,
    #[serde(default)]
    pub attempted: Vec<String>,
    pub response: Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    ContextWindow,
    ContentPolicy,
    Ordinary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chain {
    pub state: ChainState,
    #[serde(default)]
    pub targets: Vec<Target>,
    #[serde(default)]
    pub has_unattempted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainState {
    Absent,
    Unresolved,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Ordinary {
    Absent,
    Standard {
        chain: Chain,
    },
    NonStandard {
        chain: Chain,
        #[serde(default)]
        client_targets: Vec<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    pub model_group: String,
    #[serde(default)]
    pub deployment_orders: Vec<Option<i64>>,
    #[serde(default)]
    pub current_target: Option<i64>,
    pub external: External,
    #[serde(default)]
    pub external_targets: Vec<Target>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum External {
    Absent,
    Targets,
    ClientFallbacks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weighted {
    #[serde(default)]
    pub enabled: bool,
    pub strategy: String,
    #[serde(default)]
    pub failed_deployment_id: Option<String>,
    pub model_group: String,
    #[serde(default)]
    pub deployment_ids: Vec<String>,
    #[serde(default)]
    pub cooldown_ids: Vec<String>,
    pub metadata_key: MetadataKey,
    #[serde(default)]
    pub prior_excluded_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataKey {
    Metadata,
    LitellmMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    #[serde(default)]
    pub has_choices: bool,
    pub first_finish_reason: Option<String>,
    pub function_name: String,
    pub refusal: Refusal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    NotRefusal,
    WithoutStopDetails,
    WithStopDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub model: String,
    #[serde(default)]
    pub target_order: Option<i64>,
    #[serde(default)]
    pub excluded_deployment_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub successor: Successor,
    pub fallback: FallbackAvailability,
    pub refusal: RefusalAvailability,
    pub raise_decision: RaiseDecision,
    pub metadata: MetadataOutput,
}

// the ids are a set in LiteLLM, so both sides report them sorted
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataOutput {
    pub bucket: MetadataKey,
    pub excluded_deployment_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackAvailability {
    pub default_fallbacks: bool,
    pub content_policy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefusalAvailability {
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiseDecision {
    pub content_policy: bool,
    pub anthropic_refusal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Successor {
    Stop,
    Run {
        targets: Vec<Target>,
        client_fallbacks_after: bool,
    },
}

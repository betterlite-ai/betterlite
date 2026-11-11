use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};
use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(pub Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub state: State,
    #[serde(default)]
    pub snapshot: State,
    pub config: PlanConfig,
    pub request: RequestChains,
    #[serde(default)]
    pub attempted: Vec<FallbackTarget>,
    #[serde(default)]
    pub record_selection: Option<String>,
    #[serde(default)]
    pub record_disable: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub lookup_model_group: Option<String>,
    #[serde(default)]
    pub metadata: Bucket,
    #[serde(default)]
    pub litellm_metadata: Bucket,
    #[serde(default)]
    pub disable_fallbacks: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bucket {
    #[serde(default)]
    pub present: bool,
    #[serde(default)]
    pub pre_routing_selected_model: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub original_model_group: Option<String>,
    #[serde(default)]
    pub disable_fallbacks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanConfig {
    #[serde(default)]
    pub fallbacks: Vec<FallbackChainEntry>,
    #[serde(default)]
    pub context_window_fallbacks: Vec<FallbackChainEntry>,
    #[serde(default)]
    pub content_policy_fallbacks: Vec<FallbackChainEntry>,
    pub max_fallbacks: i64,
    #[serde(default)]
    pub depth: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestChains {
    #[serde(default)]
    pub fallbacks: Option<Vec<FallbackChainEntry>>,
    #[serde(default)]
    pub context_window_fallbacks: Option<Vec<FallbackChainEntry>>,
    #[serde(default)]
    pub content_policy_fallbacks: Option<Vec<FallbackChainEntry>>,
    #[serde(default)]
    pub num_retries: Option<i64>,
    #[serde(default)]
    pub model_group_retry_policy: Option<JsonValue>,
}

// the enum in mod.rs would otherwise carry every field of this output inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(pub Box<PlanOutput>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanOutput {
    pub lookup_groups: Vec<String>,
    pub ordinary: ChainOutput,
    pub context_window: ChainOutput,
    pub content_policy: ChainOutput,
    pub disabled: bool,
    pub budget: BudgetOutput,
    pub metadata: BucketOutput,
    pub litellm_metadata: BucketOutput,
    pub carried: Option<String>,
    pub hop: HopOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainOutput {
    pub resolved: bool,
    pub targets: Vec<FallbackTarget>,
    pub has_unattempted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetOutput {
    pub depth: i64,
    pub max_fallbacks: i64,
    pub exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BucketOutput {
    pub pre_routing_selected_model: Option<String>,
    pub disable_fallbacks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HopOutput {
    pub model: String,
    pub fallbacks: Option<Vec<FallbackChainEntry>>,
    pub context_window_fallbacks: Option<Vec<FallbackChainEntry>>,
    pub content_policy_fallbacks: Option<Vec<FallbackChainEntry>>,
    pub num_retries: Option<i64>,
    pub model_group_retry_policy: Option<JsonValue>,
}

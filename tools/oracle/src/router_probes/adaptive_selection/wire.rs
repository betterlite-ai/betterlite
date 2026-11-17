use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::adaptive_selection::pre_routing::RoutingDecision;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Check {
    InitialCells,
    ApplyDelta,
    NormalizedCost,
    Pick,
    PickModel,
    PreRouting,
    Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    pub(super) check: Check,
    #[serde(default)]
    pub(super) available_models: Vec<String>,
    #[serde(default)]
    pub(super) preferences: Vec<PreferenceInput>,
    #[serde(default)]
    pub(super) request_types: Option<Vec<RequestType>>,
    #[serde(default)]
    pub(super) cells: Vec<CellInput>,
    #[serde(default)]
    pub(super) deltas: Vec<DeltaInput>,
    #[serde(default)]
    pub(super) costs: Vec<CostInput>,
    #[serde(default)]
    pub(super) cost_checks: Vec<CostCheckInput>,
    #[serde(default)]
    pub(super) weights: Option<WeightsInput>,
    #[serde(default)]
    pub(super) samples: Vec<String>,
    #[serde(default)]
    pub(super) min_quality_tier: Option<i64>,
    #[serde(default)]
    pub(super) request_type: Option<RequestType>,
    #[serde(default)]
    pub(super) messages: Option<Vec<Message>>,
    #[serde(default)]
    pub(super) headers: Option<Json>,
    #[serde(default)]
    pub(super) metadata: Option<Json>,
    #[serde(default)]
    pub(super) metadata_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreferenceInput {
    pub(super) model: String,
    pub(super) quality_tier: u8,
    #[serde(default)]
    pub(super) strengths: Option<Vec<RequestType>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CellInput {
    pub(super) model: String,
    pub(super) alpha: String,
    pub(super) beta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeltaInput {
    pub(super) request_type: RequestType,
    pub(super) model: String,
    pub(super) alpha: String,
    pub(super) beta: String,
    pub(super) delta_alpha: String,
    pub(super) delta_beta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostInput {
    pub(super) model: String,
    pub(super) amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostCheckInput {
    pub(super) model: String,
    pub(super) amount: String,
    pub(super) all_costs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WeightsInput {
    pub(super) quality: Option<String>,
    pub(super) cost: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fields {
    pub(super) cells: Option<Vec<CellOutput>>,
    pub(super) selected: Option<String>,
    pub(super) scores: Option<Vec<ScoreOutput>>,
    pub(super) cost_scores: Option<Vec<CostScoreOutput>>,
    pub(super) snapshot: Option<SnapshotOutput>,
    pub(super) routing: Option<RoutingOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CellOutput {
    pub(super) request_type: RequestType,
    pub(super) model: String,
    pub(super) alpha: String,
    pub(super) beta: String,
    pub(super) samples: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ScoreOutput {
    pub(super) model: String,
    pub(super) quality: String,
    pub(super) score: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CostScoreOutput {
    pub(super) model: String,
    pub(super) cost_score: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WeightsOutput {
    pub(super) quality: String,
    pub(super) cost: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CostOutput {
    pub(super) model: String,
    pub(super) amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SnapshotOutput {
    pub(super) router_name: String,
    pub(super) available_models: Vec<String>,
    pub(super) weights: WeightsOutput,
    pub(super) model_costs: Vec<CostOutput>,
    pub(super) cells: Vec<SnapshotCellOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SnapshotCellOutput {
    pub(super) request_type: RequestType,
    pub(super) model: String,
    pub(super) alpha: String,
    pub(super) beta: String,
    pub(super) samples: u64,
    pub(super) quality_mean: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RoutingOutput {
    pub(super) model: String,
    pub(super) messages: Option<Vec<Message>>,
    pub(super) routing_decision: RoutingDecision,
    pub(super) metadata: Json,
    pub(super) request_type: RequestType,
    pub(super) min_quality_tier: Option<i64>,
}

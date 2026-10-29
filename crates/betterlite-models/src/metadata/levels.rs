use serde::{Deserialize, Serialize};

/// Reasoning effort level.
///
/// The schema declares three near-duplicate value sets (`default_reasoning_effort`,
/// `reasoning_effort_levels` items and `bedrock_output_config_effort_ceiling`); the pinned map
/// never puts a value outside its own field's subset, so one superset enum covers all three
/// without three near-identical types.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

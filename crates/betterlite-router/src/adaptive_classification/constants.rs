pub const DEFAULT_QUALITY_WEIGHT: f64 = 0.7;
pub const DEFAULT_COST_WEIGHT: f64 = 0.3;
pub const BASE_TIER_WEIGHT: [(u8, f64); 3] = [(1, 0.3), (2, 0.5), (3, 0.7)];
pub const STRENGTH_BONUS: f64 = 0.3;
pub const COLD_START_MASS: f64 = 10.0;
pub const SAMPLE_CAP: usize = 200;
pub const MIN_TURNS_FOR_CLEAN_CREDIT: u32 = 3;
pub const OWNER_CACHE_TTL_SECONDS: u64 = 86_400;
pub const SIGNAL_GATE_MIN_MESSAGES: usize = 4;
pub const MISALIGNMENT_JACCARD_THRESHOLD: f64 = 0.45;
pub const STAGNATION_JACCARD_NEAR_DUP: f64 = 0.5;
pub const LOOP_REPEAT_THRESHOLD: u32 = 3;
pub const TOOL_CALL_HISTORY_MAX: usize = 20;

pub const MIN_QUALITY_TIER_HEADER: &str = "x-litellm-min-quality-tier";
pub const MIN_QUALITY_TIER_METADATA_KEY: &str = "min_quality_tier";
pub const CHOSEN_MODEL_KEY: &str = "adaptive_router_chosen_model";
pub const ADAPTIVE_MODEL_HEADER: &str = "x-litellm-adaptive-router-model";

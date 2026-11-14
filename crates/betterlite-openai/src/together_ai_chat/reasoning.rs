use serde_json::Value;

use super::{TogetherCapabilities, TogetherFlag};
use crate::openai_base_config::Object;

const ADJUSTABLE_REASONING_MODELS: [&str; 2] = ["openai/gpt-oss-120b", "openai/gpt-oss-20b"];
const HYBRID_REASONING_MODELS: [&str; 7] = [
    "MiniMaxAI/MiniMax-M3",
    "Qwen/Qwen3.5-9B",
    "Qwen/Qwen3.6-Plus",
    "deepseek-ai/DeepSeek-V4-Pro",
    "moonshotai/Kimi-K3",
    "nvidia/nemotron-3-ultra-550b-a55b",
    "zai-org/GLM-5.2",
];
const HIGH_MAX_EFFORT_MODEL_PREFIX: &str = "deepseek-ai/DeepSeek-V4-Pro";
const EFFORT_TRANSLATION: [(&str, &str); 3] =
    [("minimal", "low"), ("xhigh", "high"), ("max", "high")];
const HIGH_MAX_EFFORT_TRANSLATION: [(&str, &str); 4] = [
    ("minimal", "high"),
    ("low", "high"),
    ("medium", "high"),
    ("xhigh", "max"),
];
const REASONING_EFFORT: &str = "reasoning_effort";
const DEFAULT_EFFORT: &str = "default";
const NO_EFFORT: &str = "none";
const LOW_EFFORT: &str = "low";

#[must_use]
pub fn supports_together_reasoning(model: &str, capabilities: &dyn TogetherCapabilities) -> bool {
    if ADJUSTABLE_REASONING_MODELS.contains(&model)
        || HYBRID_REASONING_MODELS.contains(&model)
        || model.starts_with(HIGH_MAX_EFFORT_MODEL_PREFIX)
    {
        return true;
    }
    capabilities
        .check(TogetherFlag::SupportsReasoning, model)
        .unwrap_or_else(|error| {
            tracing::debug!(
                flag = TogetherFlag::SupportsReasoning.name(),
                model,
                error = %error,
                "Error checking together_ai capability"
            );
            false
        })
}

// LiteLLM removes reasoning_effort first and then fills the payload with setdefault, so a caller-set key wins
pub(super) fn rewrite_reasoning_effort(
    optional_params: &mut Object,
    model: &str,
    capabilities: &dyn TogetherCapabilities,
) {
    let Some(Value::String(effort)) = optional_params.get(REASONING_EFFORT) else {
        return;
    };
    let effort = effort.clone();
    optional_params.remove(REASONING_EFFORT);
    for (key, value) in reasoning_payload(&effort, model, capabilities) {
        optional_params.entry(key).or_insert(value);
    }
}

fn reasoning_payload(effort: &str, model: &str, capabilities: &dyn TogetherCapabilities) -> Object {
    if effort == DEFAULT_EFFORT {
        return Object::new();
    }
    if ADJUSTABLE_REASONING_MODELS.contains(&model) {
        let adjusted = if effort == NO_EFFORT {
            tracing::debug!(
                model,
                "together_ai model cannot disable reasoning; mapping reasoning_effort=none to low"
            );
            LOW_EFFORT
        } else {
            translated(effort, &EFFORT_TRANSLATION)
        };
        return Object::from_iter([(
            String::from(REASONING_EFFORT),
            Value::String(adjusted.to_owned()),
        )]);
    }
    if effort == NO_EFFORT {
        let mut reasoning = Object::new();
        reasoning.insert(String::from("enabled"), Value::Bool(false));
        return Object::from_iter([(String::from("reasoning"), Value::Object(reasoning))]);
    }
    let declared = capabilities
        .declared_reasoning_efforts(model)
        .unwrap_or_default();
    if declared.iter().any(|declared| declared == effort) {
        return Object::from_iter([(
            String::from(REASONING_EFFORT),
            Value::String(effort.to_owned()),
        )]);
    }
    let table = if model.starts_with(HIGH_MAX_EFFORT_MODEL_PREFIX) {
        &HIGH_MAX_EFFORT_TRANSLATION[..]
    } else {
        &EFFORT_TRANSLATION[..]
    };
    Object::from_iter([(
        String::from(REASONING_EFFORT),
        Value::String(translated(effort, table).to_owned()),
    )])
}

fn translated<'effort>(effort: &'effort str, table: &[(&str, &'static str)]) -> &'effort str {
    table
        .iter()
        .find_map(|(source, target)| (*source == effort).then_some(*target))
        .unwrap_or(effort)
}

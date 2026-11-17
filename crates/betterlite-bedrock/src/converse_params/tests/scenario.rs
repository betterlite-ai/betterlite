use betterlite_anthropic::anthropic_capability_catalog::DropParams;

use crate::model_routes::ParamMap;

use super::support::{Answers, Call, Fixture, Outcome, Session, params, run};

pub(super) const MODEL_COST: &str = r#"{
    "anthropic.claude-sonnet-4-20250514-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "anthropic.claude-opus-4-1-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": false},
    "anthropic.claude-4-5-bedrock": {"litellm_provider": "bedrock", "supports_prompt_caching": true, "cache_creation_input_token_cost_above_1hr": 0.0000024},
    "anthropic.claude-3-5-sonnet": {"litellm_provider": "bedrock", "supports_prompt_caching": true},
    "anthropic.claude-no-cache": {"litellm_provider": "bedrock", "supports_prompt_caching": false},
    "anthropic.claude-adaptive": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true},
    "amazon.nova-pro-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "amazon.nova-2-lite-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "cohere.command-r-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": false},
    "deepseek.v3-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "deepseek.r1-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "openai.gpt-5": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "openai.gpt-oss-120b-1:0": {"litellm_provider": "bedrock"}
}"#;

pub(super) const ANTHROPIC: &str = "anthropic.claude-sonnet-4-20250514-v1:0";
pub(super) const KEEP: DropParams = DropParams {
    global: false,
    call: false,
};
pub(super) const DROP_CALL: DropParams = DropParams {
    global: false,
    call: true,
};
pub(super) const DROP_GLOBAL: DropParams = DropParams {
    global: true,
    call: false,
};

pub(super) fn mapped(
    fixture: &Fixture,
    session: &mut Session<'_>,
    answers: &Answers,
    model: &str,
    non_default: &str,
) -> Outcome {
    run(
        fixture,
        session,
        answers,
        Call {
            model,
            non_default: params(non_default),
            optional: ParamMap::new(),
            drop: KEEP,
        },
    )
}

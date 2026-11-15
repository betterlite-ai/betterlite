use betterlite_openai::openai_base_config::{ModelFamily, azure_model_family};

use super::support::{gpt5_supported, o_series_supported, params, resolves_none};
use crate::chat_params::capabilities::model_map_lookup_name;

const GPT5_WITH_TOP_P: &str = "max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice";
const GPT5_WITH_USER: &str = "max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format user reasoning_effort verbosity";
const GPT5_PLAIN: &str = "max_tokens max_completion_tokens n seed stream stream_options temperature tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice";
const GPT5_2: &str = "logprobs top_logprobs max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice logprobs top_logprobs";
const GPT5_2_PRO: &str = "max_tokens max_completion_tokens n seed stream stream_options temperature tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice logprobs top_logprobs";
const O_SERIES_WITH_USER: &str = "logit_bias max_tokens max_completion_tokens modalities prediction n seed stop stream stream_options temperature tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format user reasoning_effort";
const O_SERIES_NON_REASONING: &str = "logit_bias max_tokens max_completion_tokens modalities prediction n seed stop stream stream_options temperature tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format user";
const O_SERIES_PLAIN: &str = "logit_bias max_tokens max_completion_tokens modalities prediction n seed stop stream stream_options temperature tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort";

const LOOKUP_NAMES: [(&str, &str); 4] = [
    ("gpt5_series/gpt-5.1", "azure/gpt-5.1"),
    ("azure/gpt-5.1", "azure/gpt-5.1"),
    ("gpt-5.1", "azure/gpt-5.1"),
    ("gpt5_series/gpt5_series/team-x", "azure/gpt5_series/team-x"),
];
const FAMILIES: [(&str, ModelFamily); 7] = [
    ("gpt5_series/gpt-5.1", ModelFamily::GptReasoning),
    ("azure/gpt-5.1", ModelFamily::GptReasoning),
    ("gpt-5.1", ModelFamily::GptReasoning),
    ("team/gpt5_series-deploy", ModelFamily::GptReasoning),
    ("gpt-6.0", ModelFamily::GptReasoning),
    ("gpt-5-chat-latest", ModelFamily::Gpt),
    ("my-gpt5-deployment", ModelFamily::Gpt),
];
const ALIAS_ALLOWLISTS: [(&str, &str); 4] = [
    ("gpt5_series/gpt-5.1", GPT5_WITH_TOP_P),
    ("azure/gpt-5.1", GPT5_WITH_USER),
    ("gpt-5.1", GPT5_WITH_USER),
    ("my-gpt5-deployment", GPT5_PLAIN),
];
const GPT5_ALLOWLISTS: [(&str, &str); 6] = [
    ("gpt5_series/gpt-5.1", GPT5_WITH_TOP_P),
    ("gpt5_series/gpt-5.2", GPT5_2),
    ("gpt5_series/gpt-5.2-pro", GPT5_2_PRO),
    ("gpt5_series/gpt-5.3", GPT5_PLAIN),
    ("my-gpt5-deployment", GPT5_PLAIN),
    ("azure/gpt-5.1", GPT5_WITH_USER),
];
const O_SERIES_ALLOWLISTS: [(&str, &str); 5] = [
    ("o1-mini", O_SERIES_WITH_USER),
    ("gpt-4o", O_SERIES_NON_REASONING),
    ("my-o1-deployment", O_SERIES_PLAIN),
    ("o3-mini", O_SERIES_PLAIN),
    ("o_series/my-o1", O_SERIES_PLAIN),
];

pub(super) fn gpt5_alias_catalog() {
    for (model, expected) in LOOKUP_NAMES {
        assert_eq!(model_map_lookup_name(model), expected);
    }
    for (model, expected) in FAMILIES {
        assert_eq!(azure_model_family(model), expected);
    }
    alias_allowlists_and_none_effort();
}

fn alias_allowlists_and_none_effort() {
    for (model, expected) in ALIAS_ALLOWLISTS {
        assert_eq!(gpt5_supported(model), params(expected));
    }
    for model in ["gpt5_series/gpt-5.1", "azure/gpt-5.1", "gpt-5.1"] {
        assert!(resolves_none(model));
    }
    assert!(!resolves_none("my-gpt5-deployment"));
}

pub(super) fn o_series_base_policy_composition() {
    for (model, expected) in O_SERIES_ALLOWLISTS {
        assert_eq!(o_series_supported(model), params(expected));
    }
}

pub(super) fn azure_gpt5_allowlist() {
    for (model, expected) in GPT5_ALLOWLISTS {
        assert_eq!(gpt5_supported(model), params(expected));
    }
}

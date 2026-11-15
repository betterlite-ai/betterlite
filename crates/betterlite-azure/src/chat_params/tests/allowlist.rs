use betterlite_openai::openai_base_config::{ModelFamily, azure_model_family};

use super::support::{gpt5_supported, o_series_supported, params, resolves_none};
use crate::chat_params::capabilities::model_map_lookup_name;

pub(super) fn gpt5_alias_catalog() {
    assert_eq!(
        model_map_lookup_name("gpt5_series/gpt-5.1"),
        "azure/gpt-5.1"
    );
    assert_eq!(model_map_lookup_name("azure/gpt-5.1"), "azure/gpt-5.1");
    assert_eq!(model_map_lookup_name("gpt-5.1"), "azure/gpt-5.1");
    assert_eq!(
        model_map_lookup_name("gpt5_series/gpt5_series/team-x"),
        "azure/gpt5_series/team-x"
    );
    assert_eq!(
        azure_model_family("gpt5_series/gpt-5.1"),
        ModelFamily::GptReasoning
    );
    assert_eq!(
        azure_model_family("azure/gpt-5.1"),
        ModelFamily::GptReasoning
    );
    assert_eq!(azure_model_family("gpt-5.1"), ModelFamily::GptReasoning);
    assert_eq!(
        azure_model_family("team/gpt5_series-deploy"),
        ModelFamily::GptReasoning
    );
    assert_eq!(azure_model_family("gpt-6.0"), ModelFamily::GptReasoning);
    assert_eq!(azure_model_family("gpt-5-chat-latest"), ModelFamily::Gpt);
    assert_eq!(azure_model_family("my-gpt5-deployment"), ModelFamily::Gpt);
    assert_eq!(
        gpt5_supported("gpt5_series/gpt-5.1"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice"
        )
    );
    assert_eq!(
        gpt5_supported("azure/gpt-5.1"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format user reasoning_effort verbosity"
        )
    );
    assert_eq!(
        gpt5_supported("gpt-5.1"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format user reasoning_effort verbosity"
        )
    );
    assert_eq!(
        gpt5_supported("my-gpt5-deployment"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice"
        )
    );
    assert!(resolves_none("gpt5_series/gpt-5.1"));
    assert!(resolves_none("azure/gpt-5.1"));
    assert!(resolves_none("gpt-5.1"));
    assert!(!resolves_none("my-gpt5-deployment"));
}

pub(super) fn o_series_base_policy_composition() {
    assert_eq!(
        o_series_supported("o1-mini"),
        params(
            "logit_bias max_tokens max_completion_tokens modalities prediction n seed stop stream stream_options temperature tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format user reasoning_effort"
        )
    );
    assert_eq!(
        o_series_supported("gpt-4o"),
        params(
            "logit_bias max_tokens max_completion_tokens modalities prediction n seed stop stream stream_options temperature tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format user"
        )
    );
    assert_eq!(
        o_series_supported("my-o1-deployment"),
        params(
            "logit_bias max_tokens max_completion_tokens modalities prediction n seed stop stream stream_options temperature tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort"
        )
    );
    assert_eq!(
        o_series_supported("o3-mini"),
        params(
            "logit_bias max_tokens max_completion_tokens modalities prediction n seed stop stream stream_options temperature tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort"
        )
    );
    assert_eq!(
        o_series_supported("o_series/my-o1"),
        params(
            "logit_bias max_tokens max_completion_tokens modalities prediction n seed stop stream stream_options temperature tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls audio web_search_options service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort"
        )
    );
}

pub(super) fn azure_gpt5_allowlist() {
    assert_eq!(
        gpt5_supported("gpt5_series/gpt-5.1"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice"
        )
    );
    assert_eq!(
        gpt5_supported("gpt5_series/gpt-5.2"),
        params(
            "logprobs top_logprobs max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice logprobs top_logprobs"
        )
    );
    assert_eq!(
        gpt5_supported("gpt5_series/gpt-5.2-pro"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice logprobs top_logprobs"
        )
    );
    assert_eq!(
        gpt5_supported("gpt5_series/gpt-5.3"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice"
        )
    );
    assert_eq!(
        gpt5_supported("my-gpt5-deployment"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature tools function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format reasoning_effort verbosity tool_choice"
        )
    );
    assert_eq!(
        gpt5_supported("azure/gpt-5.1"),
        params(
            "max_tokens max_completion_tokens n seed stream stream_options temperature top_p tools tool_choice function_call functions max_retries extra_headers parallel_tool_calls service_tier safety_identifier prompt_cache_key prompt_cache_retention store response_format user reasoning_effort verbosity"
        )
    );
}

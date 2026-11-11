#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::{
    CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag, ParameterCapabilities,
    supported_parameters,
};
use crate::identity::provider_id::ProviderId;
use crate::params::CallType;

const BASE: &[&str] = &[
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "top_logprobs",
    "max_tokens",
    "max_completion_tokens",
    "modalities",
    "prediction",
    "n",
    "presence_penalty",
    "seed",
    "stop",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "tools",
    "tool_choice",
    "function_call",
    "functions",
    "max_retries",
    "extra_headers",
    "parallel_tool_calls",
    "audio",
    "web_search_options",
    "service_tier",
    "safety_identifier",
    "prompt_cache_key",
    "prompt_cache_retention",
    "store",
];

fn caps(names: &[OpenAiCapability]) -> BTreeSet<OpenAiCapability> {
    names.iter().copied().collect()
}

fn query<'model>(
    model: &'model str,
    config: OpenAiConfigTag,
    capabilities: &'model BTreeSet<OpenAiCapability>,
) -> CapabilityQuery<'model> {
    CapabilityQuery {
        provider: ProviderId::Openai,
        call_type: CallType::ChatCompletion,
        config,
        model: ModelProjection {
            model,
            capabilities,
        },
        base_model: None,
    }
}

fn listed(names: &[&'static str]) -> ParameterCapabilities {
    ParameterCapabilities::Listed(names.to_vec())
}

fn gpt_4o_list() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let result = supported_parameters(&query("gpt-4o", OpenAiConfigTag::Gpt, &capabilities));
    assert_eq!(
        result,
        listed(&[BASE, &["response_format", "user"]].concat())
    );
}

fn gpt_4_omits_response_format() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let result = supported_parameters(&query("gpt-4", OpenAiConfigTag::Gpt, &capabilities));
    assert_eq!(result, listed(&[BASE, &["user"]].concat()));
}

fn gpt_3_5_turbo_16k_omits_response_format() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let result = supported_parameters(&query(
        "gpt-3.5-turbo-16k",
        OpenAiConfigTag::Gpt,
        &capabilities,
    ));
    assert_eq!(result, listed(&[BASE, &["user"]].concat()));
}

fn non_catalog_model_gets_no_user() {
    let capabilities = caps(&[]);
    let result = supported_parameters(&query("my-deployment", OpenAiConfigTag::Gpt, &capabilities));
    assert_eq!(result, listed(&[BASE, &["response_format"]].concat()));
}

fn unknown_model_adds_reasoning_effort() {
    let capabilities = caps(&[]);
    let result = supported_parameters(&query(
        "my-deployment",
        OpenAiConfigTag::UnknownModel,
        &capabilities,
    ));
    assert_eq!(
        result,
        listed(&[BASE, &["response_format", "reasoning_effort"]].concat())
    );
}

fn o_series_removes_sampling_parameters() {
    let capabilities = caps(&[
        OpenAiCapability::CatalogModel,
        OpenAiCapability::FunctionCalling,
        OpenAiCapability::ResponseSchema,
    ]);
    let result = supported_parameters(&query("o3", OpenAiConfigTag::OSeries, &capabilities));
    assert_eq!(
        result,
        listed(&[
            "logit_bias",
            "max_tokens",
            "max_completion_tokens",
            "modalities",
            "prediction",
            "n",
            "seed",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "tools",
            "tool_choice",
            "function_call",
            "functions",
            "max_retries",
            "extra_headers",
            "audio",
            "web_search_options",
            "service_tier",
            "safety_identifier",
            "prompt_cache_key",
            "prompt_cache_retention",
            "store",
            "response_format",
            "user",
            "reasoning_effort",
        ])
    );
}

fn o_series_keeps_parallel_tool_calls_when_supported() {
    let capabilities = caps(&[
        OpenAiCapability::CatalogModel,
        OpenAiCapability::FunctionCalling,
        OpenAiCapability::ParallelToolCalls,
        OpenAiCapability::ResponseSchema,
    ]);
    let result = supported_parameters(&query("o1-pro", OpenAiConfigTag::OSeries, &capabilities));
    assert_eq!(
        result,
        listed(&[
            "logit_bias",
            "max_tokens",
            "max_completion_tokens",
            "modalities",
            "prediction",
            "n",
            "seed",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "tools",
            "tool_choice",
            "function_call",
            "functions",
            "max_retries",
            "extra_headers",
            "parallel_tool_calls",
            "audio",
            "web_search_options",
            "service_tier",
            "safety_identifier",
            "prompt_cache_key",
            "prompt_cache_retention",
            "store",
            "response_format",
            "user",
            "reasoning_effort",
        ])
    );
}

fn o_series_capability_flags_gate_tools_and_schema() {
    let capabilities = caps(&[
        OpenAiCapability::CatalogModel,
        OpenAiCapability::ParallelToolCalls,
    ]);
    let result = supported_parameters(&query("o3", OpenAiConfigTag::OSeries, &capabilities));
    assert_eq!(
        result,
        listed(&[
            "logit_bias",
            "max_tokens",
            "max_completion_tokens",
            "modalities",
            "prediction",
            "n",
            "seed",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "max_retries",
            "extra_headers",
            "parallel_tool_calls",
            "audio",
            "web_search_options",
            "service_tier",
            "safety_identifier",
            "prompt_cache_key",
            "prompt_cache_retention",
            "store",
            "user",
            "reasoning_effort",
        ])
    );
}

fn gpt_5_removes_sampling_and_penalty_parameters() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel, OpenAiCapability::ToolChoice]);
    let result = supported_parameters(&query("gpt-5", OpenAiConfigTag::Gpt5, &capabilities));
    assert_eq!(
        result,
        listed(&[
            "max_tokens",
            "max_completion_tokens",
            "n",
            "seed",
            "stream",
            "stream_options",
            "temperature",
            "tools",
            "tool_choice",
            "function_call",
            "functions",
            "max_retries",
            "extra_headers",
            "parallel_tool_calls",
            "service_tier",
            "safety_identifier",
            "prompt_cache_key",
            "prompt_cache_retention",
            "store",
            "response_format",
            "user",
            "reasoning_effort",
            "verbosity",
        ])
    );
}

fn gpt_5_keeps_sampling_when_none_effort_is_supported() {
    let capabilities = caps(&[
        OpenAiCapability::CatalogModel,
        OpenAiCapability::ToolChoice,
        OpenAiCapability::NoneReasoningEffort,
    ]);
    let result = supported_parameters(&query("gpt-5.1", OpenAiConfigTag::Gpt5, &capabilities));
    assert_eq!(
        result,
        listed(&[
            "logprobs",
            "top_logprobs",
            "max_tokens",
            "max_completion_tokens",
            "n",
            "seed",
            "stream",
            "stream_options",
            "temperature",
            "top_p",
            "tools",
            "tool_choice",
            "function_call",
            "functions",
            "max_retries",
            "extra_headers",
            "parallel_tool_calls",
            "service_tier",
            "safety_identifier",
            "prompt_cache_key",
            "prompt_cache_retention",
            "store",
            "response_format",
            "user",
            "reasoning_effort",
            "verbosity",
        ])
    );
}

fn gpt_5_drops_tool_choice_when_unsupported() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let result = supported_parameters(&query("gpt-5", OpenAiConfigTag::Gpt5, &capabilities));
    assert_eq!(
        result,
        listed(&[
            "max_tokens",
            "max_completion_tokens",
            "n",
            "seed",
            "stream",
            "stream_options",
            "temperature",
            "tools",
            "function_call",
            "functions",
            "max_retries",
            "extra_headers",
            "parallel_tool_calls",
            "service_tier",
            "safety_identifier",
            "prompt_cache_key",
            "prompt_cache_retention",
            "store",
            "response_format",
            "user",
            "reasoning_effort",
            "verbosity",
        ])
    );
}

fn gpt_5_search_uses_its_own_list() {
    let capabilities = caps(&[
        OpenAiCapability::CatalogModel,
        OpenAiCapability::ToolChoice,
        OpenAiCapability::NoneReasoningEffort,
    ]);
    let result = supported_parameters(&query(
        "gpt-5-search-api",
        OpenAiConfigTag::Gpt5,
        &capabilities,
    ));
    assert_eq!(
        result,
        listed(&[
            "max_tokens",
            "max_completion_tokens",
            "stream",
            "stream_options",
            "web_search_options",
            "service_tier",
            "safety_identifier",
            "response_format",
            "user",
            "store",
            "verbosity",
            "max_retries",
            "extra_headers",
        ])
    );
}

fn union(model: &str, base: &str) -> ParameterCapabilities {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut query = query(model, OpenAiConfigTag::Gpt, &capabilities);
    query.base_model = Some(ModelProjection {
        model: base,
        capabilities: &capabilities,
    });
    supported_parameters(&query)
}

fn base_model_adds_names_in_first_seen_order() {
    assert_eq!(
        union("gpt-4", "gpt-4o"),
        listed(&[BASE, &["user", "response_format"]].concat())
    );
}

fn identical_base_model_adds_nothing() {
    assert_eq!(
        union("gpt-4o", "gpt-4o"),
        listed(&[BASE, &["response_format", "user"]].concat())
    );
    let capabilities = caps(&[]);
    let base_capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut query = query("gpt-4o", OpenAiConfigTag::Gpt, &capabilities);
    query.base_model = Some(ModelProjection {
        model: "gpt-4o",
        capabilities: &base_capabilities,
    });
    assert_eq!(
        supported_parameters(&query),
        listed(&[BASE, &["response_format"]].concat())
    );
}

fn embeddings_ignores_the_base_model() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut query = query("gpt-4", OpenAiConfigTag::Gpt, &capabilities);
    query.call_type = CallType::Embeddings;
    query.base_model = Some(ModelProjection {
        model: "gpt-4o",
        capabilities: &capabilities,
    });
    assert_eq!(
        supported_parameters(&query),
        listed(&[BASE, &["user"]].concat())
    );
}

fn embeddings_returns_the_chat_parameter_names() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut query = query("gpt-4o", OpenAiConfigTag::Gpt, &capabilities);
    query.call_type = CallType::Embeddings;
    assert_eq!(
        supported_parameters(&query),
        listed(&[BASE, &["response_format", "user"]].concat())
    );
}

fn unimplemented_provider_is_absent() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut query = query("claude-3", OpenAiConfigTag::Gpt, &capabilities);
    query.provider = ProviderId::Anthropic;
    assert_eq!(supported_parameters(&query), ParameterCapabilities::Absent);
}

fn transcription_is_absent() {
    let capabilities = caps(&[OpenAiCapability::CatalogModel]);
    let mut query = query("gpt-4o-transcribe", OpenAiConfigTag::Gpt, &capabilities);
    query.call_type = CallType::Transcription;
    assert_eq!(supported_parameters(&query), ParameterCapabilities::Absent);
}

#[rstest]
#[case::gpt_4o_list(gpt_4o_list)]
#[case::gpt_4_omits_response_format(gpt_4_omits_response_format)]
#[case::gpt_3_5_turbo_16k_omits_response_format(gpt_3_5_turbo_16k_omits_response_format)]
#[case::non_catalog_model_gets_no_user(non_catalog_model_gets_no_user)]
#[case::unknown_model_adds_reasoning_effort(unknown_model_adds_reasoning_effort)]
#[case::o_series_removes_sampling_parameters(o_series_removes_sampling_parameters)]
#[case::o_series_keeps_parallel_tool_calls_when_supported(
    o_series_keeps_parallel_tool_calls_when_supported
)]
#[case::o_series_capability_flags_gate_tools_and_schema(
    o_series_capability_flags_gate_tools_and_schema
)]
#[case::gpt_5_removes_sampling_and_penalty_parameters(
    gpt_5_removes_sampling_and_penalty_parameters
)]
#[case::gpt_5_keeps_sampling_when_none_effort_is_supported(
    gpt_5_keeps_sampling_when_none_effort_is_supported
)]
#[case::gpt_5_drops_tool_choice_when_unsupported(gpt_5_drops_tool_choice_when_unsupported)]
#[case::gpt_5_search_uses_its_own_list(gpt_5_search_uses_its_own_list)]
#[case::base_model_adds_names_in_first_seen_order(base_model_adds_names_in_first_seen_order)]
#[case::identical_base_model_adds_nothing(identical_base_model_adds_nothing)]
#[case::embeddings_ignores_the_base_model(embeddings_ignores_the_base_model)]
#[case::embeddings_returns_the_chat_parameter_names(embeddings_returns_the_chat_parameter_names)]
#[case::unimplemented_provider_is_absent(unimplemented_provider_is_absent)]
#[case::transcription_is_absent(transcription_is_absent)]
fn provider_params_tests_behavior(#[case] scenario: fn()) {
    scenario();
}

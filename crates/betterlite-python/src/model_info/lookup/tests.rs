#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_metadata_resolution::error::ADD_MODEL_URL;
use indexmap::IndexMap;
use pyo3_async_runtimes::tokio::get_runtime;

use super::openai_params::is_openai_model;
use super::{max_tokens, model_info, provider_info};
use crate::model_info::convert::{self, ModelQuery, ProviderQuery, RuntimeEntries};
use crate::model_info::error::ModelInfoError;
use crate::model_info::state::Runtime;
use crate::native_api::bundled_model_cost_json;

fn runtime() -> Runtime {
    let text = bundled_model_cost_json().unwrap();
    Runtime::new(&text).unwrap()
}

fn query(model: &str) -> ModelQuery {
    ModelQuery {
        model: model.to_owned(),
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    }
}

fn provider_query(model: &str, provider: &str) -> ProviderQuery {
    ProviderQuery {
        model: model.to_owned(),
        custom_llm_provider: Some(provider.to_owned()),
    }
}

fn fields(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn number(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn parameter_names(info: &IndexMap<String, Json>) -> Vec<String> {
    let Some(Json::List(names)) = info.get("supported_openai_params") else {
        panic!("supported_openai_params is not a list");
    };
    names
        .iter()
        .map(|name| match name {
            Json::Text(text) => text.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                panic!("a supported parameter name is not text")
            }
        })
        .collect()
}

#[test]
fn model_info_lookup_returns_the_openai_entry() {
    let runtime = runtime();
    let text = get_runtime()
        .block_on(model_info(&runtime, &query("gpt-4")))
        .unwrap();
    let info = fields(&text);
    assert_eq!(
        info.get("litellm_provider"),
        Some(&Json::Text("openai".to_owned()))
    );
    assert_eq!(info.get("max_tokens"), Some(&number("4096")));
    let Some(Json::List(parameters)) = info.get("supported_openai_params") else {
        panic!("supported_openai_params is not a list");
    };
    assert!(parameters.contains(&Json::Text("user".to_owned())));
    assert!(parameters.contains(&Json::Text("temperature".to_owned())));
}

#[test]
fn model_info_lookup_gpt_5_and_o_series_parameters() {
    let runtime = runtime();
    let gpt_5 = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("gpt-5")))
            .unwrap(),
    ));
    assert!(gpt_5.iter().any(|name| name == "reasoning_effort"));
    assert!(gpt_5.iter().any(|name| name == "verbosity"));
    assert!(!gpt_5.iter().any(|name| name == "logprobs"));
    assert!(!gpt_5.iter().any(|name| name == "top_p"));
    let o3 = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("o3")))
            .unwrap(),
    ));
    assert!(o3.iter().any(|name| name == "reasoning_effort"));
    assert!(o3.iter().any(|name| name == "tools"));
    assert!(!o3.iter().any(|name| name == "logprobs"));
    assert!(!o3.iter().any(|name| name == "parallel_tool_calls"));
    assert!(gpt_5.iter().any(|name| name == "user"));
    let mut gpt_5_without_user = gpt_5;
    gpt_5_without_user.retain(|name| name != "user");
    let prefixed_gpt_5 = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/gpt-5")))
            .unwrap(),
    ));
    assert_eq!(prefixed_gpt_5, gpt_5_without_user);
    assert!(o3.iter().any(|name| name == "user"));
    let mut o3_without_user = o3;
    o3_without_user.retain(|name| name != "user");
    let prefixed_o3 = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/o3")))
            .unwrap(),
    ));
    assert_eq!(prefixed_o3, o3_without_user);
}

#[test]
fn model_info_lookup_reads_capability_flags_from_a_runtime_entry() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"o3":{"litellm_provider":"openai","mode":"chat","max_tokens":100000,"supports_function_calling":false,"supports_response_schema":true,"supports_tool_choice":true}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("o3")))
            .unwrap(),
    ));
    assert!(!parameters.iter().any(|name| name == "tools"));
    assert!(!parameters.iter().any(|name| name == "tool_choice"));
    assert!(!parameters.iter().any(|name| name == "function_call"));
    assert!(!parameters.iter().any(|name| name == "functions"));
    assert!(parameters.iter().any(|name| name == "reasoning_effort"));
    assert!(parameters.iter().any(|name| name == "response_format"));
}

#[test]
fn model_info_lookup_reads_gpt_5_flags_from_a_runtime_entry() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"gpt-5-custom":{"litellm_provider":"openai","mode":"chat","max_tokens":4096,"supports_tool_choice":true,"supports_none_reasoning_effort":true}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("gpt-5-custom")))
            .unwrap(),
    ));
    assert!(parameters.iter().any(|name| name == "tool_choice"));
    assert!(parameters.iter().any(|name| name == "logprobs"));
    assert!(parameters.iter().any(|name| name == "top_p"));
    assert!(!parameters.iter().any(|name| name == "user"));
}

#[test]
fn model_info_lookup_reads_capability_flags_from_a_prefixed_runtime_entry() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/o3":{"litellm_provider":"openai","mode":"chat","max_tokens":100000,"supports_function_calling":false,"supports_response_schema":true,"supports_tool_choice":true}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/o3")))
            .unwrap(),
    ));
    assert!(!parameters.iter().any(|name| name == "tools"));
    assert!(!parameters.iter().any(|name| name == "tool_choice"));
    assert!(!parameters.iter().any(|name| name == "function_call"));
    assert!(!parameters.iter().any(|name| name == "functions"));
    assert!(parameters.iter().any(|name| name == "reasoning_effort"));
    assert!(parameters.iter().any(|name| name == "response_format"));
}

#[test]
fn model_info_lookup_reads_a_prefixed_entry_for_the_inferred_provider() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/o3":{"litellm_provider":"openai","supports_function_calling":false}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("o3")))
            .unwrap(),
    ));
    assert!(!parameters.iter().any(|name| name == "tools"));
    assert!(parameters.iter().any(|name| name == "response_format"));
}

#[test]
fn model_info_lookup_falls_back_to_the_bare_entry_for_a_missing_flag() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/o3":{"litellm_provider":"openai","mode":"chat","max_tokens":100000}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/o3")))
            .unwrap(),
    ));
    assert!(parameters.iter().any(|name| name == "tools"));
    assert!(parameters.iter().any(|name| name == "tool_choice"));
    assert!(parameters.iter().any(|name| name == "response_format"));
}

#[test]
fn model_info_lookup_falls_back_to_the_bare_entry_for_parallel_tool_calls() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/o1-2024-12-17":{"litellm_provider":"openai","mode":"chat","max_tokens":100000},"o1-2024-12-17":{"litellm_provider":"openai","mode":"chat","max_tokens":100000,"supports_parallel_function_calling":true}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/o1-2024-12-17")))
            .unwrap(),
    ));
    assert!(parameters.iter().any(|name| name == "parallel_tool_calls"));
}

#[test]
fn model_info_lookup_falls_back_to_the_bare_entry_for_none_reasoning_effort() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/gpt-5.1":{"litellm_provider":"openai","mode":"chat","max_tokens":4096},"gpt-5.1":{"litellm_provider":"openai","mode":"chat","max_tokens":4096,"supports_none_reasoning_effort":true}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/gpt-5.1")))
            .unwrap(),
    ));
    assert!(parameters.iter().any(|name| name == "logprobs"));
    assert!(parameters.iter().any(|name| name == "top_p"));
}

#[test]
fn model_info_lookup_reads_parallel_tool_calls_from_a_synced_flag() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/o1-2024-12-17":{"litellm_provider":"openai","mode":"chat","max_tokens":100000,"supports_parallel_function_calling":true},"o1-2024-12-17":{"litellm_provider":"openai","mode":"chat","max_tokens":100000,"supports_parallel_function_calling":false}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/o1-2024-12-17")))
            .unwrap(),
    ));
    assert!(parameters.iter().any(|name| name == "parallel_tool_calls"));
}

#[test]
fn model_info_lookup_keeps_an_explicit_parallel_flag_over_the_bare_entry() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/o1-2024-12-17":{"litellm_provider":"openai","mode":"chat","max_tokens":100000,"supports_parallel_function_calling":false},"o1-2024-12-17":{"litellm_provider":"openai","mode":"chat","max_tokens":100000,"supports_parallel_function_calling":true}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/o1-2024-12-17")))
            .unwrap(),
    ));
    assert!(!parameters.iter().any(|name| name == "parallel_tool_calls"));
}

#[test]
fn model_info_lookup_skips_a_prefixed_entry_for_another_provider() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/o3":{"litellm_provider":"azure","mode":"chat","max_tokens":100000,"supports_function_calling":false}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let openai_query = ModelQuery {
        model: "o3".to_owned(),
        custom_llm_provider: Some("openai".to_owned()),
        api_base: None,
        api_key: None,
    };
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &openai_query))
            .unwrap(),
    ));
    assert!(parameters.iter().any(|name| name == "tools"));
    assert!(parameters.iter().any(|name| name == "response_format"));
}

#[test]
fn model_info_lookup_reads_gpt_5_flags_from_a_prefixed_runtime_entry() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"openai/gpt-5":{"litellm_provider":"openai","mode":"chat","max_tokens":4096,"supports_tool_choice":false,"supports_none_reasoning_effort":true}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let parameters = parameter_names(&fields(
        &get_runtime()
            .block_on(model_info(&runtime, &query("openai/gpt-5")))
            .unwrap(),
    ));
    assert!(!parameters.iter().any(|name| name == "tool_choice"));
    assert!(parameters.iter().any(|name| name == "logprobs"));
    assert!(parameters.iter().any(|name| name == "top_p"));
    assert!(!parameters.iter().any(|name| name == "user"));
}

#[test]
fn model_info_lookup_resolves_a_prefixed_bedrock_id() {
    let runtime = runtime();
    let model = "bedrock/us.anthropic.claude-3-5-haiku-20241022-v1:0";
    let text = get_runtime()
        .block_on(model_info(&runtime, &query(model)))
        .unwrap();
    let info = fields(&text);
    assert_eq!(
        info.get("litellm_provider"),
        Some(&Json::Text("bedrock".to_owned()))
    );
    assert_eq!(info.get("max_tokens"), Some(&number("8192")));
    assert_eq!(info.get("supported_openai_params"), Some(&Json::Null));
}

#[test]
fn model_info_lookup_resolves_a_vertex_model() {
    let runtime = runtime();
    let model = "vertex_ai/gemini-3-pro-preview";
    let text = get_runtime()
        .block_on(model_info(&runtime, &query(model)))
        .unwrap();
    let info = fields(&text);
    assert_eq!(
        info.get("litellm_provider"),
        Some(&Json::Text("vertex_ai".to_owned()))
    );
    assert_eq!(info.get("max_tokens"), Some(&number("65535")));
}

#[test]
fn model_info_lookup_resolves_a_fine_tune_name() {
    let runtime = runtime();
    let text = get_runtime()
        .block_on(model_info(&runtime, &query("ft:gpt-4o-2024-08-06")))
        .unwrap();
    let info = fields(&text);
    assert_eq!(
        info.get("litellm_provider"),
        Some(&Json::Text("openai".to_owned()))
    );
    assert_eq!(info.get("max_tokens"), Some(&number("16384")));
}

#[test]
fn model_info_lookup_raises_the_unmapped_message() {
    let runtime = runtime();
    let error = get_runtime()
        .block_on(model_info(&runtime, &query("not-a-real-model-xyz")))
        .unwrap_err();
    assert!(matches!(error, ModelInfoError::Lookup(_)));
    assert_eq!(
        error.to_string(),
        format!(
            "This model isn't mapped yet. model=not-a-real-model-xyz, custom_llm_provider=None. Add it here - {ADD_MODEL_URL}."
        )
    );
}

#[test]
fn model_info_lookup_max_tokens_reads_the_exact_entry() {
    let runtime = runtime();
    let found = get_runtime()
        .block_on(max_tokens(&runtime, "gpt-4"))
        .unwrap();
    assert_eq!(found, Some(4096));
}

#[test]
fn model_info_lookup_max_tokens_reads_the_resolved_name() {
    let runtime = runtime();
    let found = get_runtime()
        .block_on(max_tokens(&runtime, "openai/gpt-4"))
        .unwrap();
    assert_eq!(found, Some(4096));
}

#[test]
fn model_info_lookup_max_tokens_prefers_max_output_tokens() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"acme-tokens":{"litellm_provider":"openai","max_tokens":3,"max_output_tokens":7}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let found = get_runtime()
        .block_on(max_tokens(&runtime, "acme-tokens"))
        .unwrap();
    assert_eq!(found, Some(7));
}

#[test]
fn model_info_lookup_max_tokens_keeps_a_null_key_null() {
    let runtime = runtime();
    let entries: RuntimeEntries = convert::parse(
        r#"{"entries":{"acme-null":{"litellm_provider":"openai","max_output_tokens":null,"max_tokens":5}},"removed":[]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(entries);
    let found = get_runtime()
        .block_on(max_tokens(&runtime, "acme-null"))
        .unwrap();
    assert_eq!(found, None);
}

#[test]
fn model_info_lookup_max_tokens_returns_null_for_an_entry_without_keys() {
    let runtime = runtime();
    let entries: RuntimeEntries =
        convert::parse(r#"{"entries":{"gpt-4":{"litellm_provider":"openai"}},"removed":[]}"#)
            .unwrap();
    runtime.apply_runtime_entries(entries);
    let found = get_runtime()
        .block_on(max_tokens(&runtime, "openai/gpt-4"))
        .unwrap();
    assert_eq!(found, None);
}

#[test]
fn model_info_lookup_max_tokens_raises_for_a_resolved_name_without_an_entry() {
    let runtime = runtime();
    let error = get_runtime()
        .block_on(max_tokens(&runtime, "openai/acme-absent-model"))
        .unwrap_err();
    assert!(matches!(error, ModelInfoError::MaxTokens { .. }));
}

#[test]
fn model_info_lookup_reads_the_provider_from_a_runtime_entry() {
    let runtime = runtime();
    let entries: RuntimeEntries =
        convert::parse(r#"{"entries":{"acme-probe":{"litellm_provider":"openai"}},"removed":[]}"#)
            .unwrap();
    runtime.apply_runtime_entries(entries);
    let state = runtime.state();
    let resolution = runtime.resolution(&state);
    assert!(is_openai_model(
        &resolution,
        &state.entries,
        "acme-probe",
        None
    ));
}

#[test]
fn model_info_lookup_keeps_the_explicit_provider_over_a_conflicting_entry() {
    let runtime = runtime();
    let entries: RuntimeEntries =
        convert::parse(r#"{"entries":{"gpt-4":{"litellm_provider":"azure"}},"removed":[]}"#)
            .unwrap();
    runtime.apply_runtime_entries(entries);
    let state = runtime.state();
    let resolution = runtime.resolution(&state);
    assert!(is_openai_model(
        &resolution,
        &state.entries,
        "gpt-4",
        Some("openai"),
    ));
}

#[test]
fn model_info_lookup_max_tokens_raises_the_unmapped_message() {
    let runtime = runtime();
    let error = get_runtime()
        .block_on(max_tokens(&runtime, "not-a-real-model-xyz"))
        .unwrap_err();
    assert!(matches!(error, ModelInfoError::MaxTokens { .. }));
    assert_eq!(
        error.to_string(),
        format!("Model not-a-real-model-xyz isn't mapped yet. Add it here - {ADD_MODEL_URL}")
    );
}

#[test]
fn model_info_lookup_provider_info_reads_the_xai_flags() {
    let runtime = runtime();
    let text = provider_info(&runtime, &provider_query("grok-3", "xai"))
        .unwrap()
        .unwrap();
    assert_eq!(text, r#"{"supports_web_search":true}"#);
}

#[test]
fn model_info_lookup_provider_info_reads_the_fireworks_flags() {
    let runtime = runtime();
    let model = "fireworks_ai/accounts/fireworks/models/llama-v3p1-8b-instruct";
    let text = provider_info(&runtime, &provider_query(model, "fireworks_ai"))
        .unwrap()
        .unwrap();
    assert_eq!(
        text,
        r#"{"supports_function_calling":false,"supports_prompt_caching":true}"#
    );
}

#[test]
fn model_info_lookup_provider_info_returns_none_for_openai() {
    let runtime = runtime();
    assert_eq!(
        provider_info(&runtime, &provider_query("gpt-4", "openai")).unwrap(),
        None
    );
}

#[test]
fn model_info_lookup_provider_info_returns_none_without_a_provider() {
    let runtime = runtime();
    let query = ProviderQuery {
        model: "gpt-4".to_owned(),
        custom_llm_provider: None,
    };
    assert_eq!(provider_info(&runtime, &query).unwrap(), None);
}

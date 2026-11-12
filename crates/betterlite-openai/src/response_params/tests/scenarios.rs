use std::collections::BTreeSet;

use betterlite_models::identity::provider_id::ProviderId;
use serde_json::{Map, Value};

use super::super::capabilities::{CUSTOM_LLM_PROVIDER, provider_capabilities};
use super::super::decode::{ResponseDecodeError, decode_response_capability_error};
use super::super::params::supported_openai_params;
use super::{BundledReasoningModels, Probe, assert_mapped, rejected};

const GPT_5_CATALOG: &str = r#"{
    "gpt-5.2": {"litellm_provider": "openai", "supports_reasoning": true, "supports_none_reasoning_effort": true, "default_reasoning_effort": "none"},
    "gpt-5": {"litellm_provider": "openai", "supports_reasoning": true, "supports_none_reasoning_effort": false},
    "gpt-4o": {"litellm_provider": "openai"},
    "gpt-5-chat-latest": {"litellm_provider": "openai"}
}"#;

const MINIMUM_CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai"},
    "gpt-5.2": {"litellm_provider": "openai", "supports_reasoning": true, "supports_none_reasoning_effort": true}
}"#;

const REASONING_CATALOG: &str = r#"{
    "gpt-5.2": {"litellm_provider": "openai", "supports_reasoning": false, "supports_none_reasoning_effort": true, "default_reasoning_effort": "none"},
    "gpt-5.2-reasoning": {"litellm_provider": "openai", "supports_reasoning": true, "supports_none_reasoning_effort": true}
}"#;

const SUMMARY_CATALOG: &str = r#"{
    "gpt-5.2": {"litellm_provider": "openai", "supports_reasoning": false, "supports_none_reasoning_effort": true, "default_reasoning_effort": "none"},
    "gpt-5": {"litellm_provider": "openai", "supports_reasoning": true, "supports_none_reasoning_effort": false}
}"#;

const WITHOUT_DEFAULTS: &str = r#"{"gpt-5.2": {"litellm_provider": "openai", "supports_reasoning": true, "supports_none_reasoning_effort": true}}"#;
const DECLARED_ELSEWHERE: &str = r#"{"gpt-5.2": {"litellm_provider": "openai", "supports_reasoning": true, "supports_none_reasoning_effort": true}, "unrelated": {"litellm_provider": "openai", "default_reasoning_effort": "high"}}"#;
const NONE_UNSUPPORTED: &str = r#"{"gpt-5.2": {"litellm_provider": "openai", "supports_reasoning": true, "supports_none_reasoning_effort": false, "default_reasoning_effort": "none"}}"#;

#[derive(Clone, Copy)]
enum Expected {
    Mapped(&'static str),
    Echoed,
    Error(&'static str),
    Dropped(&'static str),
}

type Row = (&'static str, &'static str, &'static str, Expected);

#[rustfmt::skip]
const MINIMUM_ROWS: [Row; 12] = [
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": 1}"#, Expected::Mapped(r#"{"max_output_tokens": 16}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": 15}"#, Expected::Mapped(r#"{"max_output_tokens": 16}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": 16}"#, Expected::Mapped(r#"{"max_output_tokens": 16}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": 0}"#, Expected::Mapped(r#"{"max_output_tokens": 16}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": -3}"#, Expected::Mapped(r#"{"max_output_tokens": 16}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": 17}"#, Expected::Mapped(r#"{"max_output_tokens": 17}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": 18446744073709551615}"#, Expected::Mapped(r#"{"max_output_tokens": 18446744073709551615}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": 15.5}"#, Expected::Mapped(r#"{"max_output_tokens": 15.5}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": "8"}"#, Expected::Mapped(r#"{"max_output_tokens": "8"}"#)),
    (MINIMUM_CATALOG, "gpt-4o", r#"{"max_output_tokens": null}"#, Expected::Mapped(r#"{"max_output_tokens": null}"#)),
    (MINIMUM_CATALOG, "gpt-5.2", r#"{"max_output_tokens": 1}"#, Expected::Mapped(r#"{"max_output_tokens": 16}"#)),
    (MINIMUM_CATALOG, "gpt-4o", "{}", Expected::Mapped("{}")),
];

#[rustfmt::skip]
const REASONING_ROWS: [Row; 9] = [
    (REASONING_CATALOG, "gpt-5.2", r#"{"reasoning": {"effort": "high"}}"#, Expected::Error("reasoning.effort")),
    (REASONING_CATALOG, "gpt-5.2", r#"{"reasoning": {"effort": "high"}}"#, Expected::Dropped("{}")),
    (REASONING_CATALOG, "gpt-5.2", r#"{"reasoning": {"summary": "auto"}}"#, Expected::Echoed),
    (REASONING_CATALOG, "gpt-5.2", r#"{"reasoning": {"effort": null}}"#, Expected::Echoed),
    (REASONING_CATALOG, "gpt-5.2", r#"{"reasoning": "high"}"#, Expected::Echoed),
    (REASONING_CATALOG, "gpt-5.2", r#"{"reasoning": null}"#, Expected::Echoed),
    (REASONING_CATALOG, "gpt-5.2", r#"{"reasoning": {}}"#, Expected::Echoed),
    (REASONING_CATALOG, "gpt-5.2-reasoning", r#"{"reasoning": {"effort": "high"}}"#, Expected::Echoed),
    (REASONING_CATALOG, "gpt-5.2-reasoning", r#"{"reasoning": {"effort": "low"}}"#, Expected::Echoed),
];

#[rustfmt::skip]
const GPT5_TEMPERATURE_ROWS: [Row; 10] = [
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 0.5}"#, Expected::Mapped(r#"{"temperature": 0.5}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 2, "reasoning": {"effort": "high"}}"#, Expected::Error("temperature=2")),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 2, "reasoning": {"effort": "high"}}"#, Expected::Dropped(r#"{"reasoning": {"effort": "high"}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 2, "reasoning": {"effort": "none"}}"#, Expected::Mapped(r#"{"temperature": 2, "reasoning": {"effort": "none"}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 1, "reasoning": {"effort": "high"}}"#, Expected::Mapped(r#"{"temperature": 1, "reasoning": {"effort": "high"}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 1.0, "reasoning": {"effort": "high"}}"#, Expected::Mapped(r#"{"temperature": 1.0, "reasoning": {"effort": "high"}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": null, "reasoning": {"effort": "high"}}"#, Expected::Mapped(r#"{"temperature": null, "reasoning": {"effort": "high"}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 2, "reasoning": {"effort": null}}"#, Expected::Mapped(r#"{"temperature": 2, "reasoning": {"effort": null}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 2}"#, Expected::Mapped(r#"{"temperature": 2}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 2, "reasoning": {"effort": 1}}"#, Expected::Error("temperature=2")),
];

#[rustfmt::skip]
const GPT5_MODEL_ROWS: [Row; 7] = [
    (WITHOUT_DEFAULTS, "gpt-5.2", r#"{"temperature": 2}"#, Expected::Mapped(r#"{"temperature": 2}"#)),
    (DECLARED_ELSEWHERE, "gpt-5.2", r#"{"temperature": 2}"#, Expected::Error("temperature=2")),
    (GPT_5_CATALOG, "gpt-5", r#"{"temperature": 2}"#, Expected::Error("temperature=2")),
    (NONE_UNSUPPORTED, "gpt-5.2", r#"{"temperature": 2}"#, Expected::Error("temperature=2")),
    (GPT_5_CATALOG, "openai/gpt-5.2", r#"{"temperature": 2}"#, Expected::Mapped(r#"{"temperature": 2}"#)),
    (GPT_5_CATALOG, "gpt-5-chat-latest", r#"{"temperature": 2}"#, Expected::Mapped(r#"{"temperature": 2}"#)),
    (GPT_5_CATALOG, "azure/gpt-5.2", r#"{"temperature": 2, "reasoning": {"effort": "high"}}"#, Expected::Mapped(r#"{"temperature": 2, "reasoning": {"effort": "high"}}"#)),
];

#[rustfmt::skip]
const TOP_P_ROWS: [Row; 13] = [
    (GPT_5_CATALOG, "gpt-5.2", r#"{"top_p": 0.9}"#, Expected::Mapped(r#"{"top_p": 0.9}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"temperature": 0.5}"#, Expected::Mapped(r#"{"temperature": 0.5}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"top_p": 0.9, "reasoning": {"effort": "high"}}"#, Expected::Error("top_p")),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"top_p": 0.9, "reasoning": {"effort": "high"}}"#, Expected::Dropped(r#"{"reasoning": {"effort": "high"}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"top_p": null, "reasoning": {"effort": "high"}}"#, Expected::Error("top_p")),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"top_p": null, "reasoning": {"effort": "high"}}"#, Expected::Dropped(r#"{"reasoning": {"effort": "high"}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"top_p": null, "reasoning": {"effort": "none"}}"#, Expected::Mapped(r#"{"top_p": null, "reasoning": {"effort": "none"}}"#)),
    (GPT_5_CATALOG, "gpt-5.2", r#"{"top_p": 0.9, "reasoning": {"summary": "auto"}}"#, Expected::Mapped(r#"{"top_p": 0.9, "reasoning": {"summary": "auto"}}"#)),
    (GPT_5_CATALOG, "gpt-4o", r#"{"top_p": 0.9}"#, Expected::Mapped(r#"{"top_p": 0.9}"#)),
    (GPT_5_CATALOG, "gpt-5-chat-latest", r#"{"top_p": 0.9}"#, Expected::Mapped(r#"{"top_p": 0.9}"#)),
    (GPT_5_CATALOG, "gpt-5", r#"{"top_p": 0.9}"#, Expected::Error("top_p")),
    (GPT_5_CATALOG, "gpt-5", r#"{"top_p": 0.9}"#, Expected::Dropped("{}")),
    (GPT_5_CATALOG, "azure/gpt-5.2", r#"{"top_p": 0.9, "reasoning": {"effort": "high"}}"#, Expected::Mapped(r#"{"top_p": 0.9, "reasoning": {"effort": "high"}}"#)),
];

#[rustfmt::skip]
const UNKNOWN_ROWS: [Row; 11] = [
    (r#"{"mystery-reasoner": {"litellm_provider": "openai", "supports_reasoning": true}}"#, "mystery-reasoner", r#"{"reasoning": {"effort": "high"}}"#, Expected::Echoed),
    (r#"{"mystery-reasoner": {"litellm_provider": "openai", "supports_reasoning": false}}"#, "mystery-reasoner", r#"{"reasoning": {"effort": "high"}}"#, Expected::Error("reasoning.effort")),
    (r#"{"mystery-reasoner": {"litellm_provider": "openai"}}"#, "mystery-reasoner", r#"{"reasoning": {"effort": "high"}}"#, Expected::Error("reasoning.effort")),
    (r#"{"gpt-5.2": {"litellm_provider": "openai"}}"#, "gpt-5.2", r#"{"reasoning": {"effort": "high"}}"#, Expected::Echoed),
    (r#"{"gpt-5.2": {"litellm_provider": "openai", "supports_reasoning": false}}"#, "gpt-5.2", r#"{"reasoning": {"effort": "high"}}"#, Expected::Error("reasoning.effort")),
    (r#"{"gpt-5.2": {}}"#, "openai/gpt-5.2", r#"{"reasoning": {"effort": "high"}}"#, Expected::Echoed),
    ("{}", "unlisted-reasoner", r#"{"reasoning": {"effort": "high"}}"#, Expected::Echoed),
    (r#"{"o3-mini": {}}"#, "o3-mini", r#"{"reasoning": {"effort": "high"}}"#, Expected::Echoed),
    (r#"{"gpt-4o": {"litellm_provider": "openai"}}"#, "gpt-4o", r#"{"reasoning": {"effort": "high"}}"#, Expected::Error("reasoning.effort")),
    (r#"{"deepseek-reasoner": {}}"#, "deepseek-reasoner", r#"{"reasoning": {"effort": "high"}}"#, Expected::Error("reasoning.effort")),
    (r#"{"deepseek-reasoner": {"litellm_provider": "openai"}}"#, "deepseek-reasoner", r#"{"reasoning": {"effort": "high"}}"#, Expected::Error("reasoning.effort")),
];

#[rustfmt::skip]
const SUMMARY_ROWS: [Row; 8] = [
    (SUMMARY_CATALOG, "gpt-5.2", r#"{"reasoning": {"summary": "auto"}}"#, Expected::Echoed),
    (SUMMARY_CATALOG, "gpt-5.2", r#"{"reasoning": {"summary": "auto", "generate_summary": "auto"}}"#, Expected::Echoed),
    (SUMMARY_CATALOG, "gpt-5.2", r#"{"reasoning": "high"}"#, Expected::Echoed),
    (SUMMARY_CATALOG, "gpt-5.2", r#"{"reasoning": [1, 2]}"#, Expected::Echoed),
    (SUMMARY_CATALOG, "gpt-5.2", r#"{"reasoning": {"summary": "auto"}, "temperature": 2}"#, Expected::Echoed),
    (SUMMARY_CATALOG, "gpt-5", r#"{"reasoning": {"summary": "auto"}, "temperature": 2}"#, Expected::Error("temperature=2")),
    (SUMMARY_CATALOG, "gpt-5.2", r#"{"reasoning": {"summary": "auto", "effort": "high"}}"#, Expected::Error("reasoning.effort")),
    (SUMMARY_CATALOG, "gpt-5", r#"{"reasoning": {"summary": "auto"}, "top_p": 0.9}"#, Expected::Error("top_p")),
];

fn json(source: &str) -> Value {
    serde_json::from_str(source).expect("json")
}

fn options(model: &str, params: &Value) -> Value {
    let mut object = Map::new();
    object.insert(String::from("model"), Value::String(model.to_owned()));
    object.insert(String::from("params"), params.clone());
    Value::Object(object)
}

fn azure_options(model: &str, params: &Value) -> Value {
    let mut object = Map::new();
    object.insert(
        String::from("provider"),
        Value::String(String::from("azure")),
    );
    object.insert(String::from("model"), Value::String(model.to_owned()));
    object.insert(String::from("params"), params.clone());
    Value::Object(object)
}

fn check_table(bundled: &mut BundledReasoningModels, rows: &[Row]) {
    for (catalog, model, params, expected) in rows {
        let parsed_catalog = json(catalog);
        let parsed_params = json(params);
        let probe = Probe::new(parsed_catalog.clone(), options(model, &parsed_params));
        match expected {
            Expected::Mapped(value) => assert_mapped(bundled, probe, &json(value)),
            Expected::Echoed => assert_mapped(bundled, probe, &parsed_params),
            Expected::Error(fragment) => {
                let error = rejected(bundled, probe);
                let message = error.to_string();
                assert!(message.contains(fragment), "{message}");
            }
            Expected::Dropped(value) => {
                for dropped in [
                    Probe::new(parsed_catalog.clone(), options(model, &parsed_params))
                        .dropping_request(),
                    Probe::new(parsed_catalog.clone(), options(model, &parsed_params))
                        .dropping_globally(),
                ] {
                    assert_mapped(bundled, dropped, &json(value));
                }
            }
        }
    }
}

pub(super) fn minimum_output_tokens() {
    let mut bundled = BundledReasoningModels::default();
    check_table(&mut bundled, &MINIMUM_ROWS);
}

pub(super) fn reasoning_unsupported_drop_or_error() {
    let mut bundled = BundledReasoningModels::default();
    check_table(&mut bundled, &REASONING_ROWS);
    let effort = json(r#"{"reasoning": {"effort": "high"}}"#);
    let error = rejected(
        &mut bundled,
        Probe::new(json(REASONING_CATALOG), options("gpt-5.2", &effort)),
    );
    assert!(error.to_string().contains("supports_reasoning"), "{error}");
    assert_mapped(
        &mut bundled,
        Probe::new(json(REASONING_CATALOG), azure_options("gpt-5.2", &effort)),
        &effort,
    );
}

pub(super) fn gpt5_temperature_default_effort() {
    let mut bundled = BundledReasoningModels::default();
    check_table(&mut bundled, &GPT5_TEMPERATURE_ROWS);
    check_table(&mut bundled, &GPT5_MODEL_ROWS);
    assert_mapped(
        &mut bundled,
        Probe::new(
            json(GPT_5_CATALOG),
            azure_options(
                "gpt-5.2",
                &json(r#"{"temperature": 2, "reasoning": {"effort": "high"}}"#),
            ),
        ),
        &json(r#"{"temperature": 2, "reasoning": {"effort": "high"}}"#),
    );
}

pub(super) fn top_p_with_reasoning() {
    let mut bundled = BundledReasoningModels::default();
    check_table(&mut bundled, &TOP_P_ROWS);
    assert_mapped(
        &mut bundled,
        Probe::new(
            json(GPT_5_CATALOG),
            azure_options(
                "gpt-5.2",
                &json(r#"{"top_p": 0.9, "reasoning": {"effort": "high"}}"#),
            ),
        ),
        &json(r#"{"top_p": 0.9, "reasoning": {"effort": "high"}}"#),
    );
}

pub(super) fn unknown_catalog_model() {
    let mut bundled = BundledReasoningModels::default();
    check_table(&mut bundled, &UNKNOWN_ROWS);
    let effort = json(r#"{"reasoning": {"effort": "high"}}"#);
    assert_mapped(
        &mut bundled,
        Probe::new(
            json(
                r#"{"mystery-reasoner": {"litellm_provider": "openai", "supports_reasoning": false}}"#,
            ),
            azure_options("mystery-reasoner", &effort),
        ),
        &effort,
    );
}

pub(super) fn reasoning_summary_without_effort() {
    let mut bundled = BundledReasoningModels::default();
    check_table(&mut bundled, &SUMMARY_ROWS);
}

pub(super) fn responses_parameter_set() {
    let expected: BTreeSet<&str> = [
        "input",
        "model",
        "extra_headers",
        "extra_query",
        "extra_body",
        "timeout",
        "include",
        "instructions",
        "max_output_tokens",
        "metadata",
        "parallel_tool_calls",
        "previous_response_id",
        "reasoning",
        "store",
        "background",
        "stream",
        "temperature",
        "text",
        "tool_choice",
        "tools",
        "top_p",
        "truncation",
        "user",
        "service_tier",
        "safety_identifier",
        "prompt",
        "max_tool_calls",
        "prompt_cache_key",
        "prompt_cache_retention",
        "prompt_cache_options",
        "stream_options",
        "top_logprobs",
        "partial_images",
        "context_management",
    ]
    .into_iter()
    .collect();
    assert_eq!(supported_openai_params(), expected);
    assert_eq!(supported_openai_params().len(), 34_usize);
}

pub(super) fn provider_capability_flags() {
    for (provider, expected) in [
        (
            CUSTOM_LLM_PROVIDER,
            json(
                r#"{"custom_llm_provider": "openai", "supports_native_file_search": true, "supports_encrypted_agent_messages": true}"#,
            ),
        ),
        (
            ProviderId::Azure,
            json(
                r#"{"custom_llm_provider": "azure", "supports_native_file_search": true, "supports_encrypted_agent_messages": true}"#,
            ),
        ),
        (
            ProviderId::Anthropic,
            json(
                r#"{"custom_llm_provider": "anthropic", "supports_native_file_search": true, "supports_encrypted_agent_messages": false}"#,
            ),
        ),
    ] {
        let capabilities = serde_json::to_value(provider_capabilities(provider)).expect("flags");
        assert_eq!(capabilities, expected);
    }
    decode_response_bodies();
}

fn decode_response_bodies() {
    let body = decode_response_capability_error(
        br#"{"id": "resp_1", "object": "response", "deleted": false}"#,
    )
    .expect("a JSON object");
    assert_eq!(body.get("deleted"), Some(&Value::Bool(false)));
    assert!(matches!(
        decode_response_capability_error(b"[]"),
        Err(ResponseDecodeError::NotObject)
    ));
    assert!(matches!(
        decode_response_capability_error(b"{"),
        Err(ResponseDecodeError::Json { .. })
    ));
    assert!(matches!(
        decode_response_capability_error(b""),
        Err(ResponseDecodeError::Json { .. })
    ));
}

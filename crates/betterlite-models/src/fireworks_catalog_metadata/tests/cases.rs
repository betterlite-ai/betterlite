#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use crate::entry::ModelFields;
use crate::fireworks_catalog_metadata::{
    Capability, Catalog, FireworksMetadata, MODEL_COST_KEY_PREFIX, ProviderSpecificModelInfo,
    capability_exact, resolve_resource_name,
};

#[rstest]
#[case::resource_names(resource_names)]
#[case::exact_capability(exact_capability)]
#[case::indexed_fuzzy_capability(indexed_fuzzy_capability)]
#[case::provider_defaults(provider_defaults)]
fn fireworks_catalog_metadata_cases(#[case] scenario: fn()) {
    scenario();
}

fn fields(source: &str) -> ModelFields {
    let raw: Box<RawValue> = serde_json::from_str(source).unwrap();
    ModelFields::parse(raw.as_ref()).unwrap().0
}

fn catalog_of(entries: &[(&str, &str)]) -> Catalog {
    entries
        .iter()
        .map(|(key, source)| ((*key).to_owned(), fields(source)))
        .collect()
}

fn exact_catalog() -> Catalog {
    catalog_of(&[
        ("raw-model", r#"{"supports_vision":true}"#),
        ("fireworks_ai/some-model", r#"{"supports_vision":true}"#),
        (
            "fireworks_ai/accounts/fireworks/models/some-model",
            r#"{"supports_vision":false}"#,
        ),
        ("fireworks_ai/accounts/fireworks/models/vision-less", "{}"),
        ("later-model", "{}"),
        ("fireworks_ai/later-model", r#"{"supports_vision":true}"#),
        (
            "fireworks_ai/accounts/fireworks/routers/firerouter",
            r#"{"supports_vision":true}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/glm-5p2",
            r#"{"supports_vision":false}"#,
        ),
    ])
}

fn fuzzy_catalog() -> Catalog {
    catalog_of(&[
        (
            "fireworks_ai/accounts/fireworks/models/glm-5p2",
            r#"{"supports_vision":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/qwen3-8b",
            r#"{"supports_vision":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/qwen3-8b-instruct",
            r#"{"supports_vision":true}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/some-model",
            r#"{"supports_vision":true}"#,
        ),
        ("fireworks_ai/accounts/fireworks/models/quiet-model", "{}"),
        (
            "fireworks_ai/accounts/fireworks/models/quiet-model-turbo",
            r#"{"supports_vision":true}"#,
        ),
        ("openai/gpt-4o", r#"{"supports_vision":true}"#),
        (
            "fireworks_ai/accounts/fireworks/models/",
            r#"{"supports_vision":true}"#,
        ),
    ])
}

fn ties_catalog() -> Catalog {
    catalog_of(&[
        (
            "fireworks_ai/accounts/fireworks/models/tie-b",
            r#"{"supports_vision":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/tie-a",
            r#"{"supports_vision":true}"#,
        ),
    ])
}

fn mutable_catalog() -> Catalog {
    catalog_of(&[
        (
            "fireworks_ai/accounts/fireworks/models/stable-model",
            r#"{"supports_vision":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/retiring-model",
            r#"{"supports_vision":true}"#,
        ),
        ("openai/gpt-4o", "{}"),
    ])
}

fn replacement_catalog() -> Catalog {
    catalog_of(&[
        (
            "fireworks_ai/accounts/fireworks/models/stable-model",
            r#"{"supports_vision":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/alternate-model",
            r#"{"supports_vision":true}"#,
        ),
        ("openai/gpt-4o", "{}"),
    ])
}

fn provider_catalog() -> Catalog {
    catalog_of(&[
        (
            "fireworks_ai/accounts/fireworks/models/no-tools",
            r#"{"supports_function_calling":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/reasoning-off",
            r#"{"supports_reasoning":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/reasoning-on",
            r#"{"supports_reasoning":true}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/vision-off",
            r#"{"supports_vision":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/pdf-in",
            r#"{"supports_pdf_input":true}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/caching-off",
            r#"{"supports_prompt_caching":false}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/extra-flags",
            r#"{"supports_tool_choice":true,"supports_web_search":true,"supports_mid_conversation_system":true}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/fully-known",
            r#"{"supports_function_calling":false,"supports_reasoning":true,"supports_vision":true,"supports_pdf_input":false}"#,
        ),
    ])
}

fn defaults() -> ProviderSpecificModelInfo {
    ProviderSpecificModelInfo {
        supports_function_calling: true,
        supports_prompt_caching: true,
        supports_reasoning: None,
        supports_vision: None,
        supports_pdf_input: None,
    }
}

fn model_cost_key(model: &str) -> String {
    format!("{MODEL_COST_KEY_PREFIX}{}", resolve_resource_name(model))
}

fn resource_names() {
    resource_name_prefixes();
    resource_name_preserved();
    resource_name_forwarding();
    resource_name_router_ids();
    cost_keys();
}

fn resource_name_prefixes() {
    assert_eq!(
        resolve_resource_name("glm-4p6"),
        "accounts/fireworks/models/glm-4p6"
    );
    assert_eq!(
        resolve_resource_name("fireworks_ai/glm-4p6"),
        "accounts/fireworks/models/glm-4p6"
    );
    // only one fireworks_ai/ prefix is removed
    assert_eq!(
        resolve_resource_name("fireworks_ai/fireworks_ai/glm-4p6"),
        "accounts/fireworks/models/fireworks_ai/glm-4p6"
    );
    assert_eq!(resolve_resource_name(""), "accounts/fireworks/models/");
    assert_eq!(
        resolve_resource_name("fireworks_ai/"),
        "accounts/fireworks/models/"
    );
}

fn resource_name_preserved() {
    assert_eq!(
        resolve_resource_name("accounts/fireworks/routers/glm-latest"),
        "accounts/fireworks/routers/glm-latest"
    );
    assert_eq!(
        resolve_resource_name("fireworks_ai/accounts/fireworks/models/glm-4p6"),
        "accounts/fireworks/models/glm-4p6"
    );
    assert_eq!(resolve_resource_name("FW-Kimi-K3"), "FW-Kimi-K3");
    assert_eq!(
        resolve_resource_name("fireworks_ai/FW-GLM-5.2-Fast"),
        "FW-GLM-5.2-Fast"
    );
    assert_eq!(
        resolve_resource_name("glm-4p6#accounts/gitlab/deployments/2fb7764c"),
        "glm-4p6#accounts/gitlab/deployments/2fb7764c"
    );
}

fn resource_name_forwarding() {
    assert_eq!(
        resolve_resource_name("routers/glm-latest"),
        "accounts/fireworks/routers/glm-latest"
    );
    assert_eq!(
        resolve_resource_name("models/glm-4p6"),
        "accounts/fireworks/models/glm-4p6"
    );
    assert_eq!(
        resolve_resource_name("routers/"),
        "accounts/fireworks/routers/"
    );
    assert_eq!(
        resolve_resource_name("kimi-k2p6-fast"),
        "accounts/fireworks/routers/kimi-k2p6-fast"
    );
    assert_eq!(
        resolve_resource_name("firerouter"),
        "accounts/fireworks/routers/firerouter"
    );
    assert_eq!(
        resolve_resource_name("fireworks_ai/firerouter"),
        "accounts/fireworks/routers/firerouter"
    );
    assert_eq!(
        resolve_resource_name("auto"),
        "accounts/fireworks/routers/auto"
    );
}

fn resource_name_router_ids() {
    assert_eq!(
        resolve_resource_name("fireworks_ai/auto-instant"),
        "accounts/fireworks/routers/auto-instant"
    );
    assert_eq!(
        resolve_resource_name("firerouter/kimi-k3/deepseek-v4"),
        "accounts/fireworks/routers/firerouter/kimi-k3/deepseek-v4"
    );
    assert_eq!(
        resolve_resource_name("firerouter-v2"),
        "accounts/fireworks/models/firerouter-v2"
    );
    assert_eq!(
        resolve_resource_name("autoglm-9b"),
        "accounts/fireworks/models/autoglm-9b"
    );
    assert_eq!(
        resolve_resource_name("auto-v2"),
        "accounts/fireworks/models/auto-v2"
    );
}

fn cost_keys() {
    assert_eq!(
        model_cost_key("deepseek-r1"),
        "fireworks_ai/accounts/fireworks/models/deepseek-r1"
    );
    assert_eq!(
        model_cost_key("glm-5p3-fast"),
        "fireworks_ai/accounts/fireworks/routers/glm-5p3-fast"
    );
    assert_eq!(
        model_cost_key("accounts/fireworks/models/deepseek-r1"),
        "fireworks_ai/accounts/fireworks/models/deepseek-r1"
    );
}

fn exact_capability() {
    exact_candidate_order();
    exact_family_key();
    malformed_entries_fail();
}

fn exact_candidate_order() {
    let catalog = exact_catalog();
    assert_eq!(
        capability_exact(&catalog, "raw-model", Capability::Vision),
        Some(true)
    );
    assert_eq!(
        capability_exact(&catalog, "some-model", Capability::Vision),
        Some(true)
    );
    assert_eq!(
        capability_exact(
            &catalog,
            "fireworks_ai/accounts/fireworks/models/some-model",
            Capability::Vision
        ),
        Some(false)
    );
    // a present entry whose capability is absent hands the lookup to the next candidate
    assert_eq!(
        capability_exact(&catalog, "later-model", Capability::Vision),
        Some(true)
    );
    assert_eq!(
        capability_exact(&catalog, "vision-less", Capability::Vision),
        None
    );
    // the exact lookup never falls back to a substring match
    assert_eq!(
        capability_exact(&catalog, "custom-glm-5p2", Capability::Vision),
        None
    );
}

fn exact_family_key() {
    let catalog = exact_catalog();
    assert_eq!(
        capability_exact(&catalog, "firerouter/opus", Capability::Vision),
        Some(true)
    );
    assert_eq!(
        capability_exact(&catalog, "kimi-k2p6-fast", Capability::Vision),
        None
    );
    assert_eq!(
        capability_exact(&catalog, "firerouter", Capability::Vision),
        None
    );
}

fn malformed_entries_fail() {
    // a malformed entry never reaches a lookup: the typed catalog rejects it and names the field
    let non_boolean: Box<RawValue> =
        serde_json::from_str(r#"{"supports_function_calling":"yes"}"#).unwrap();
    let error = ModelFields::parse(non_boolean.as_ref()).unwrap_err();
    assert!(error.to_string().contains("supports_function_calling"));
    let non_entry: Box<RawValue> = serde_json::from_str("5").unwrap();
    let entry_error = ModelFields::parse(non_entry.as_ref()).unwrap_err();
    assert!(entry_error.to_string().contains("invalid type"));
}

fn indexed_fuzzy_capability() {
    fuzzy_matching_rules();
    fuzzy_ties_keep_order();
    cache_follows_identity_and_generation();
}

fn fuzzy_matching_rules() {
    let catalog = fuzzy_catalog();
    let mut metadata = FireworksMetadata::default();
    // the longest matching key wins
    assert_eq!(
        metadata.get_model_cost_capability(
            &catalog,
            0,
            "custom-qwen3-8b-instruct",
            Capability::Vision
        ),
        Some(true)
    );
    // a False capability survives the fallback
    assert_eq!(
        metadata.get_model_cost_capability(&catalog, 0, "custom-glm-5p2", Capability::Vision),
        Some(false)
    );
    // a matching key whose capability is absent is skipped rather than reported
    assert_eq!(
        metadata.get_model_cost_capability(&catalog, 0, "custom-quiet-model", Capability::Vision),
        None
    );
    // a substring only counts on a hyphen boundary
    assert_eq!(
        metadata.get_model_cost_capability(&catalog, 0, "awesome-model", Capability::Vision),
        None
    );
    // a catalog key with an empty short name is left out of the index
    assert_eq!(
        metadata.get_model_cost_capability(&catalog, 0, "custom--model", Capability::Vision),
        None
    );
    // a key outside the fireworks_ai/ provider is left out of the index
    assert_eq!(
        metadata.get_model_cost_capability(&catalog, 0, "gpt-4o", Capability::Vision),
        None
    );
}

fn fuzzy_ties_keep_order() {
    let catalog = ties_catalog();
    let mut metadata = FireworksMetadata::default();
    assert_eq!(
        metadata.get_model_cost_capability(&catalog, 0, "tie-a-tie-b", Capability::Vision),
        Some(false)
    );
}

fn cache_follows_identity_and_generation() {
    let mut mutable = mutable_catalog();
    let replacement = replacement_catalog();
    let mut metadata = FireworksMetadata::default();
    assert_eq!(
        metadata.get_model_cost_capability(&mutable, 0, "custom-stable-model", Capability::Vision),
        Some(false)
    );
    mutable.insert(
        "fireworks_ai/accounts/fireworks/models/fresh-model".to_owned(),
        fields(r#"{"supports_vision":true}"#),
    );
    mutable.shift_remove("fireworks_ai/accounts/fireworks/models/retiring-model");
    // an add and a remove leave the length unchanged; without a new generation the index stays as built
    assert_eq!(
        metadata.get_model_cost_capability(&mutable, 0, "custom-fresh-model", Capability::Vision),
        None
    );
    // the generation rebuilds the index
    assert_eq!(
        metadata.get_model_cost_capability(&mutable, 1, "custom-fresh-model", Capability::Vision),
        Some(true)
    );
    // a different catalog object of the same length and generation is a different identity
    assert_eq!(
        metadata.get_model_cost_capability(
            &replacement,
            1,
            "custom-fresh-model",
            Capability::Vision
        ),
        None
    );
}

fn provider_defaults() {
    provider_info_defaults();
    provider_info_overrides();
    provider_info_fallback();
}

fn provider_info_defaults() {
    let catalog = provider_catalog();
    let mut metadata = FireworksMetadata::default();
    assert_eq!(
        metadata.get_provider_info(&catalog, 0, "unknown-model"),
        defaults()
    );
    // no flag outside the five is inferred from the catalog
    assert_eq!(
        metadata.get_provider_info(&catalog, 0, "extra-flags"),
        defaults()
    );
}

fn provider_info_overrides() {
    let catalog = provider_catalog();
    let mut metadata = FireworksMetadata::default();
    // a non-None function-calling value overrides the default even with False
    assert!(
        !metadata
            .get_provider_info(&catalog, 0, "no-tools")
            .supports_function_calling
    );
    // reasoning is included only when truthy
    assert_eq!(
        metadata
            .get_provider_info(&catalog, 0, "reasoning-off")
            .supports_reasoning,
        None
    );
    assert_eq!(
        metadata
            .get_provider_info(&catalog, 0, "reasoning-on")
            .supports_reasoning,
        Some(true)
    );
    // vision and pdf input are included when non-None, preserving False
    assert_eq!(
        metadata
            .get_provider_info(&catalog, 0, "vision-off")
            .supports_vision,
        Some(false)
    );
    assert_eq!(
        metadata
            .get_provider_info(&catalog, 0, "custom-pdf-in")
            .supports_pdf_input,
        Some(true)
    );
    // prompt caching stays true whatever the catalog row says
    assert!(
        metadata
            .get_provider_info(&catalog, 0, "caching-off")
            .supports_prompt_caching
    );
}

fn provider_info_fallback() {
    let catalog = provider_catalog();
    let mut metadata = FireworksMetadata::default();
    assert_eq!(
        metadata.get_provider_info(&catalog, 0, "custom-fully-known"),
        ProviderSpecificModelInfo {
            supports_function_calling: false,
            supports_prompt_caching: true,
            supports_reasoning: Some(true),
            supports_vision: Some(true),
            supports_pdf_input: Some(false),
        }
    );
}

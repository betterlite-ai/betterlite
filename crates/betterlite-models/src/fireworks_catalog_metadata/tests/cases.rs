#![cfg(test)]

use rstest::rstest;
use serde_json::value::RawValue;

use crate::entry::ModelFields;
use crate::fireworks_catalog_metadata::{
    Capability, Catalog, FireworksMetadata, ProviderSpecificModelInfo, capability_exact,
    get_model_cost_key, resolve_resource_name,
};

#[rstest]
#[case::resource_names(resource_names)]
#[case::exact_capability(exact_capability)]
#[case::indexed_fuzzy_capability(indexed_fuzzy_capability)]
#[case::provider_defaults(provider_defaults)]
fn fireworks_catalog_metadata_cases(#[case] scenario: fn()) {
    scenario();
}

const RESOLVED_NAMES: &[(&str, &str)] = &[
    ("glm-4p6", "accounts/fireworks/models/glm-4p6"),
    ("fireworks_ai/glm-4p6", "accounts/fireworks/models/glm-4p6"),
    (
        "fireworks_ai/fireworks_ai/glm-4p6",
        "accounts/fireworks/models/fireworks_ai/glm-4p6",
    ),
    ("", "accounts/fireworks/models/"),
    ("fireworks_ai/", "accounts/fireworks/models/"),
    (
        "accounts/fireworks/routers/glm-latest",
        "accounts/fireworks/routers/glm-latest",
    ),
    (
        "fireworks_ai/accounts/fireworks/models/glm-4p6",
        "accounts/fireworks/models/glm-4p6",
    ),
    ("FW-Kimi-K3", "FW-Kimi-K3"),
    ("fireworks_ai/FW-GLM-5.2-Fast", "FW-GLM-5.2-Fast"),
    (
        "glm-4p6#accounts/gitlab/deployments/2fb7764c",
        "glm-4p6#accounts/gitlab/deployments/2fb7764c",
    ),
    (
        "routers/glm-latest",
        "accounts/fireworks/routers/glm-latest",
    ),
    ("models/glm-4p6", "accounts/fireworks/models/glm-4p6"),
    ("routers/", "accounts/fireworks/routers/"),
    (
        "kimi-k2p6-fast",
        "accounts/fireworks/routers/kimi-k2p6-fast",
    ),
    ("firerouter", "accounts/fireworks/routers/firerouter"),
    (
        "fireworks_ai/firerouter",
        "accounts/fireworks/routers/firerouter",
    ),
    ("auto", "accounts/fireworks/routers/auto"),
    (
        "fireworks_ai/auto-instant",
        "accounts/fireworks/routers/auto-instant",
    ),
    (
        "firerouter/kimi-k3/deepseek-v4",
        "accounts/fireworks/routers/firerouter/kimi-k3/deepseek-v4",
    ),
    ("firerouter-v2", "accounts/fireworks/models/firerouter-v2"),
    ("autoglm-9b", "accounts/fireworks/models/autoglm-9b"),
    ("auto-v2", "accounts/fireworks/models/auto-v2"),
];

const COST_KEYS: &[(&str, &str)] = &[
    (
        "deepseek-r1",
        "fireworks_ai/accounts/fireworks/models/deepseek-r1",
    ),
    (
        "glm-5p3-fast",
        "fireworks_ai/accounts/fireworks/routers/glm-5p3-fast",
    ),
    (
        "accounts/fireworks/models/deepseek-r1",
        "fireworks_ai/accounts/fireworks/models/deepseek-r1",
    ),
];

const EXACT_CATALOG: &[(&str, &str)] = &[
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
    (
        "fireworks_ai/accounts/fireworks/models/deep-model",
        r#"{"supports_vision":true}"#,
    ),
];

const FUZZY_CATALOG: &[(&str, &str)] = &[
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
    (
        "fireworks_ai/accounts/fireworks/models/start-model",
        r#"{"supports_vision":true}"#,
    ),
    ("openai/gpt-4o", r#"{"supports_vision":true}"#),
    (
        "fireworks_ai/accounts/fireworks/models/",
        r#"{"supports_vision":true}"#,
    ),
];

const TIES_CATALOG: &[(&str, &str)] = &[
    (
        "fireworks_ai/accounts/fireworks/models/tie-b",
        r#"{"supports_vision":false}"#,
    ),
    (
        "fireworks_ai/accounts/fireworks/models/tie-a",
        r#"{"supports_vision":true}"#,
    ),
];

const MUTABLE_CATALOG: &[(&str, &str)] = &[
    (
        "fireworks_ai/accounts/fireworks/models/stable-model",
        r#"{"supports_vision":false}"#,
    ),
    (
        "fireworks_ai/accounts/fireworks/models/retiring-model",
        r#"{"supports_vision":true}"#,
    ),
    ("openai/gpt-4o", "{}"),
];

const REPLACEMENT_CATALOG: &[(&str, &str)] = &[
    (
        "fireworks_ai/accounts/fireworks/models/stable-model",
        r#"{"supports_vision":false}"#,
    ),
    (
        "fireworks_ai/accounts/fireworks/models/alternate-model",
        r#"{"supports_vision":true}"#,
    ),
    ("openai/gpt-4o", "{}"),
];

const PROVIDER_CATALOG: &[(&str, &str)] = &[
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
];

const DEFAULTS: ProviderSpecificModelInfo = ProviderSpecificModelInfo {
    supports_function_calling: true,
    supports_prompt_caching: true,
    supports_reasoning: None,
    supports_vision: None,
    supports_pdf_input: None,
};

const FULLY_KNOWN: ProviderSpecificModelInfo = ProviderSpecificModelInfo {
    supports_function_calling: false,
    supports_prompt_caching: true,
    supports_reasoning: Some(true),
    supports_vision: Some(true),
    supports_pdf_input: Some(false),
};

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

fn resource_names() {
    for (model, expected) in RESOLVED_NAMES {
        assert_eq!(resolve_resource_name(model), *expected);
    }
    for (model, expected) in COST_KEYS {
        assert_eq!(get_model_cost_key(model), *expected);
    }
}

fn exact_capability() {
    exact_candidate_order();
    exact_later_candidates();
    exact_family_key();
    malformed_entries_fail();
}

fn exact_candidate_order() {
    let catalog = catalog_of(EXACT_CATALOG);
    assert_exact(&catalog, "raw-model", Some(true));
    assert_exact(&catalog, "some-model", Some(true));
    // the short-key candidate answers before the accounts/fireworks/models/ candidate
    assert_exact(&catalog, "accounts/fireworks/models/some-model", Some(true));
}

fn exact_later_candidates() {
    let catalog = catalog_of(EXACT_CATALOG);
    // a present entry whose capability is absent hands the lookup to the next candidate
    assert_exact(&catalog, "later-model", Some(true));
    // the accounts/fireworks/models/ candidate answers when the short-key candidate does not
    assert_exact(&catalog, "deep-model", Some(true));
    assert_exact(&catalog, "vision-less", None);
    // the exact lookup never falls back to a substring match
    assert_exact(&catalog, "custom-glm-5p2", None);
}

fn assert_exact(catalog: &Catalog, model: &str, expected: Option<bool>) {
    assert_eq!(
        capability_exact(catalog, model, Capability::Vision),
        expected
    );
}

fn exact_family_key() {
    let catalog = catalog_of(EXACT_CATALOG);
    assert_exact(&catalog, "firerouter/opus", Some(true));
    assert_exact(&catalog, "kimi-k2p6-fast", None);
    assert_exact(&catalog, "firerouter", None);
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
    fuzzy_rejects_unrelated_substrings();
    fuzzy_matches_a_leading_word();
    fuzzy_index_leaves_out_invalid_rows();
    fuzzy_ties_keep_order();
    cache_follows_identity_and_generation();
}

fn fuzzy_matching_rules() {
    let catalog = catalog_of(FUZZY_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert_capability(
        &mut metadata,
        &catalog,
        "custom-qwen3-8b-instruct",
        Some(true),
    );
    assert_capability(&mut metadata, &catalog, "custom-glm-5p2", Some(false));
    assert_capability(&mut metadata, &catalog, "custom-quiet-model", None);
}

fn fuzzy_rejects_unrelated_substrings() {
    let catalog = catalog_of(FUZZY_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert_capability(&mut metadata, &catalog, "awesome-model", None);
}

fn fuzzy_matches_a_leading_word() {
    let catalog = catalog_of(FUZZY_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert_capability(&mut metadata, &catalog, "start-model-lite", Some(true));
}

fn fuzzy_index_leaves_out_invalid_rows() {
    let catalog = catalog_of(FUZZY_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert_capability(&mut metadata, &catalog, "custom--model", None);
    assert_capability(&mut metadata, &catalog, "gpt-4o", None);
}

fn assert_capability(
    metadata: &mut FireworksMetadata,
    catalog: &Catalog,
    model: &str,
    expected: Option<bool>,
) {
    let found = metadata.get_model_cost_capability(catalog, 0, model, Capability::Vision);
    assert_eq!(found, expected);
}

fn fuzzy_ties_keep_order() {
    let catalog = catalog_of(TIES_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert_capability(&mut metadata, &catalog, "tie-a-tie-b", Some(false));
}

fn cache_follows_identity_and_generation() {
    let mut mutable = catalog_of(MUTABLE_CATALOG);
    let replacement = catalog_of(REPLACEMENT_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert_capability(&mut metadata, &mutable, "custom-stable-model", Some(false));
    mutable.insert(
        "fireworks_ai/accounts/fireworks/models/fresh-model".to_owned(),
        fields(r#"{"supports_vision":true}"#),
    );
    mutable.shift_remove("fireworks_ai/accounts/fireworks/models/retiring-model");
    // an add and a remove leave the length unchanged; without a new generation the index stays as built
    let stale =
        metadata.get_model_cost_capability(&mutable, 0, "custom-fresh-model", Capability::Vision);
    assert_eq!(stale, None);
    // the generation rebuilds the index
    let rebuilt =
        metadata.get_model_cost_capability(&mutable, 1, "custom-fresh-model", Capability::Vision);
    assert_eq!(rebuilt, Some(true));
    // a different catalog object of the same length and generation is a different identity
    let replaced = metadata.get_model_cost_capability(
        &replacement,
        1,
        "custom-fresh-model",
        Capability::Vision,
    );
    assert_eq!(replaced, None);
}

fn provider_defaults() {
    provider_info_defaults();
    provider_info_overrides();
    provider_info_includes_optional_flags();
    provider_info_fallback();
}

fn assert_provider(
    metadata: &mut FireworksMetadata,
    catalog: &Catalog,
    model: &str,
    expected: &ProviderSpecificModelInfo,
) {
    assert_eq!(&metadata.get_provider_info(catalog, 0, model), expected);
}

fn provider_info_defaults() {
    let catalog = catalog_of(PROVIDER_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert_provider(&mut metadata, &catalog, "unknown-model", &DEFAULTS);
    // no flag outside the five is inferred from the catalog
    assert_provider(&mut metadata, &catalog, "extra-flags", &DEFAULTS);
}

fn provider_info_overrides() {
    let catalog = catalog_of(PROVIDER_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert!(
        !metadata
            .get_provider_info(&catalog, 0, "no-tools")
            .supports_function_calling
    );
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
}

fn provider_info_includes_optional_flags() {
    let catalog = catalog_of(PROVIDER_CATALOG);
    let mut metadata = FireworksMetadata::default();
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
    assert!(
        metadata
            .get_provider_info(&catalog, 0, "caching-off")
            .supports_prompt_caching
    );
}

fn provider_info_fallback() {
    let catalog = catalog_of(PROVIDER_CATALOG);
    let mut metadata = FireworksMetadata::default();
    assert_provider(&mut metadata, &catalog, "custom-fully-known", &FULLY_KNOWN);
}

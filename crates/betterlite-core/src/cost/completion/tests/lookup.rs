use std::sync::Arc;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::price_lookup::PriceLookup as _;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::tool_fees::model_info::Rate;
use betterlite_models::tool_fees::ports::ToolPriceSource as _;

use super::rig::dec;
use crate::cost::completion::lookup::{CatalogLookup, LookupSettings};
use crate::cost::context::ProviderInference as _;
use crate::cost::special::policy::ModelResolver as _;

const CATALOG: &str = r#"{
    "gpt-4o": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.001,
        "output_cost_per_token": 0.002
    },
    "tiered-chat": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.001,
        "output_cost_per_token": 0.002,
        "tiered_pricing": [
            {"input_cost_per_token": 0.001, "output_cost_per_token": 0.002, "range": [0, 1000]},
            {"input_cost_per_token": 0.003, "output_cost_per_token": 0.004, "range": [1000, 2000]}
        ]
    },
    "tool-priced": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.001,
        "output_cost_per_token": 0.002,
        "search_context_cost_per_query": {
            "search_context_size_low": 0.5,
            "search_context_size_medium": 0.75
        },
        "file_search_cost_per_gb_per_day": 1.5,
        "file_search_cost_per_1k_calls": 2.5,
        "vector_store_cost_per_gb_per_day": 3.5,
        "computer_use_input_cost_per_1k_tokens": 4.5,
        "computer_use_output_cost_per_1k_tokens": 5.5
    },
    "openai/container": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0,
        "output_cost_per_token": 0,
        "code_interpreter_cost_per_session": 6.5
    }
}"#;

fn whole(model: &str) -> String {
    model.to_owned()
}

fn unsplit(model: &str) -> (Option<&str>, &str) {
    (None, model)
}

fn lookup() -> CatalogLookup {
    CatalogLookup::new(
        CATALOG.as_bytes(),
        Arc::new(
            ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Warn)
                .expect("the fixture catalog parses"),
        ),
        LookupSettings {
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            normalizers: NameNormalizers {
                bedrock_base_model: whole,
                bedrock_routing_prefix: whole,
                mantle_region_split: unsplit,
            },
        },
    )
    .expect("the fixture catalog parses")
}

#[test]
fn an_effective_lookup_resolves_the_provider_prefixed_name() {
    let found = lookup()
        .effective_model_info("openai/gpt-4o", Some("openai"))
        .expect("the prefixed name resolves to the catalog entry");
    assert!(matches!(
        found.get("input_cost_per_token"),
        Some(Scalar::Number(_))
    ));
}

#[test]
fn a_raw_lookup_reads_only_the_exact_key() {
    let lookup = lookup();
    assert!(lookup.raw_cost_map_entry("gpt-4o").is_some());
    assert!(lookup.raw_cost_map_entry("openai/gpt-4o").is_none());
}

#[test]
fn an_unknown_model_comes_back_absent_from_every_lookup() {
    let lookup = lookup();
    assert!(lookup.effective_model_info("no-such-model", None).is_none());
    assert!(
        lookup
            .effective_priced_model("no-such-model", None)
            .is_none()
    );
    assert!(lookup.raw_cost_map_entry("no-such-model").is_none());
}

#[test]
fn a_tiered_entry_decodes_its_tiers_and_a_plain_one_has_none() {
    let lookup = lookup();
    let tiered = lookup
        .effective_priced_model("tiered-chat", Some("openai"))
        .expect("the tiered entry resolves");
    assert_eq!(tiered.tiered_pricing.len(), 2);
    let plain = lookup
        .effective_priced_model("gpt-4o", Some("openai"))
        .expect("the plain entry resolves");
    assert!(plain.tiered_pricing.is_empty());
    assert!(plain.off_peak_pricing.is_none());
    assert!(!plain.off_peak_cache_read_present);
}

// The projection LiteLLM's get_model_info returns keeps the search context and drops the other tool rates.
#[test]
fn the_tool_info_keeps_the_search_context_and_the_projection_drops_the_other_rates() {
    let info = lookup()
        .model_info("tool-priced", Some("openai"))
        .expect("the entry resolves");
    assert_eq!(info.litellm_provider.as_deref(), Some("openai"));
    assert_eq!(info.search_context.low, Rate::Value(dec("0.5")));
    assert_eq!(info.search_context.medium, Rate::Value(dec("0.75")));
    assert_eq!(info.search_context.high, Rate::Absent);
    assert_eq!(info.file_search_per_gb_per_day, Rate::Absent);
    assert_eq!(info.file_search_per_1k_calls, Rate::Absent);
    assert_eq!(info.vector_store_per_gb_per_day, Rate::Absent);
    assert_eq!(info.computer_input_per_1k, Rate::Absent);
    assert_eq!(info.computer_output_per_1k, Rate::Absent);
    assert_eq!(info.code_interpreter_per_session, Rate::Absent);
    let _error = lookup().model_info("no-such-model", None).unwrap_err();
}

#[test]
fn the_container_rate_comes_from_the_providers_container_entry() {
    let lookup = lookup();
    assert_eq!(
        lookup
            .container_session_rate("openai")
            .expect("the entry resolves"),
        Rate::Value(dec("6.5"))
    );
    let _error = lookup.container_session_rate("anthropic").unwrap_err();
}

#[test]
fn a_prefixed_model_resolves_to_its_provider_and_a_blank_one_to_none() {
    let lookup = lookup();
    let resolved = lookup
        .resolve_model("openai/gpt-4o")
        .expect("the prefix names a provider");
    assert_eq!(resolved.model, "gpt-4o");
    assert!(matches!(
        lookup.inferred_provider("openai/gpt-4o"),
        Ok(ProviderIdentity::Known(_))
    ));
    let _error = lookup.inferred_provider("").unwrap_err();
}

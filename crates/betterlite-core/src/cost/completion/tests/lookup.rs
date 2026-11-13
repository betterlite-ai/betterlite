use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::price_lookup::PriceLookup as _;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::tiered_pricing::Scalar;

use crate::cost::completion::lookup::{CatalogLookup, LookupSettings};

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

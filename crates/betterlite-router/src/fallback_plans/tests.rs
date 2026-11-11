#![cfg(test)]

use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::sections::fallback_plans::chains::{
    FallbackChainEntry, FallbackTarget, ParameterizedTarget,
};
use betterlite_config::sections::settings::JsonValue;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

use crate::fallback_plans::attempts::{AttemptedTargets, attempt_key, has_unattempted_targets};
use crate::fallback_plans::chains::{
    ChainMatch, ProviderNames, get_fallback_model_group, resolve_for_lookup_groups,
};
use crate::fallback_plans::controls::HopRequest;
use crate::fallback_plans::groups::{
    Bucket, MetadataValues, RequestMetadata, carry_pre_routing_selection,
    clear_pre_routing_selection, fallback_lookup_groups, fallbacks_disabled_for_request,
    get_pre_routing_selection, record_disable_fallbacks, record_pre_routing_selection,
};
use crate::fallback_plans::plan::{
    DepthOutcome, FallbackBudget, RequestOverrides, advance_depth, resolve_chain, select_chains,
};

const CATALOG: &str = r#"{"gpt-4": {"litellm_provider": "openai"}}"#;

struct Tables {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    registry: JSONProviderRegistry,
}

impl Tables {
    fn load() -> Self {
        Self {
            catalog: ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap(),
            names: BTreeSet::new(),
            custom_providers: BTreeSet::from(["my-llm".to_owned()]),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::load(),
        }
    }

    fn provider_names(&self) -> ProviderNames<'_> {
        ProviderNames {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: &self.registry,
        }
    }
}

fn name(target: &str) -> FallbackTarget {
    FallbackTarget::Model(target.to_owned())
}

fn parameterized(model: &str, parameters: &[(&str, JsonValue)]) -> FallbackTarget {
    FallbackTarget::Parameters(ParameterizedTarget {
        model: model.to_owned(),
        parameters: parameters
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    })
}

fn group(name: &str, targets: Vec<FallbackTarget>) -> FallbackChainEntry {
    FallbackChainEntry::Group(IndexMap::from([(name.to_owned(), targets)]))
}

fn target_models(targets: &[&FallbackTarget]) -> Vec<String> {
    targets
        .iter()
        .map(|entry| match entry {
            FallbackTarget::Model(model) => model.clone(),
            FallbackTarget::Parameters(target) => target.model.clone(),
        })
        .collect()
}

fn matched_models(matched: Option<ChainMatch<'_>>) -> Vec<String> {
    matched
        .map(|found| target_models(&found.targets))
        .unwrap_or_default()
}

fn bucket(selection: Option<&str>, routed: Option<&str>, original: Option<&str>) -> Bucket {
    Bucket {
        present: true,
        values: MetadataValues {
            pre_routing_selected_model: selection.map(str::to_owned),
            model_group: routed.map(str::to_owned),
            original_model_group: original.map(str::to_owned),
            disable_fallbacks: false,
        },
    }
}

fn request_state(metadata: Bucket, litellm_metadata: Bucket) -> RequestMetadata {
    RequestMetadata {
        metadata,
        litellm_metadata,
    }
}

#[test]
fn fallback_plans_behaviors() {
    let chains = vec![
        group("*", vec![name("backup")]),
        group("gpt-4", vec![name("claude")]),
    ];
    let tables = Tables::load();
    let names = tables.provider_names();
    let exact = get_fallback_model_group(&chains, "gpt-4", &names).unwrap();
    assert_eq!(target_models(&exact.targets), ["claude"]);
    assert!(!exact.generic);
    let generic = get_fallback_model_group(&chains, "unknown", &names).unwrap();
    assert_eq!(target_models(&generic.targets), ["backup"]);
    assert!(generic.generic);
    lookup_names_resolve(&names);
    request_chains_resolve(&names);
    selection_and_pre_routing();
}

#[rstest]
#[case::openai("openai/gpt-4", true)]
#[case::azure("azure/gpt-4", true)]
#[case::vertex_ai("vertex_ai/gpt-4", true)]
#[case::an_unlisted_provider("unknown/gpt-4", false)]
#[case::a_registry_only_provider("crusoe/gpt-4", false)]
#[case::a_custom_provider("my-llm/gpt-4", true)]
fn the_stripped_name_matches_every_provider_slug(#[case] model_group: &str, #[case] matches: bool) {
    let tables = Tables::load();
    let stripped = vec![group("gpt-4", vec![name("claude")])];
    let found = get_fallback_model_group(&stripped, model_group, &tables.provider_names());
    assert_eq!(found.is_some(), matches);
}

#[rstest]
#[case::the_inferred_provider("openai/gpt-4", true)]
#[case::another_provider("azure/gpt-4", false)]
fn the_prefixed_name_uses_the_inferred_provider(#[case] key: &str, #[case] matches: bool) {
    let tables = Tables::load();
    let prefixed = vec![group(key, vec![name("claude")])];
    let found = get_fallback_model_group(&prefixed, "gpt-4", &tables.provider_names());
    assert_eq!(found.is_some(), matches);
}

fn lookup_names_resolve(names: &ProviderNames<'_>) {
    let stripped = vec![group("gpt-4", vec![name("claude")])];
    assert_eq!(
        matched_models(get_fallback_model_group(&stripped, "openai/gpt-4", names)),
        ["claude"]
    );
    assert_eq!(
        matched_models(get_fallback_model_group(&stripped, "azure/gpt-4", names)),
        ["claude"]
    );
    let prefixed = vec![group("openai/gpt-4", vec![name("claude")])];
    assert_eq!(
        matched_models(get_fallback_model_group(&prefixed, "gpt-4", names)),
        ["claude"]
    );
    let bare = vec![FallbackChainEntry::Target(name("claude"))];
    assert_eq!(
        matched_models(get_fallback_model_group(&bare, "anything", names)),
        ["claude"]
    );
    let later_specific = vec![
        group("*", vec![name("backup")]),
        group("requested", vec![name("claude")]),
    ];
    assert_eq!(
        matched_models(resolve_for_lookup_groups(
            &later_specific,
            &["tier", "requested"],
            names
        )),
        ["claude"]
    );
}

#[test]
fn fallback_plans_boundaries() {
    let tables = Tables::load();
    let names = tables.provider_names();
    empty_chains_resolve_to_nothing(&names);
    unmatched_entries_stay_out_of_the_way(&names);
    lookup_group_selection();
    depth_bounds();
    canonical_targets_keep_distinct_parameters();
}

fn empty_chains_resolve_to_nothing(names: &ProviderNames<'_>) {
    let empty: Vec<FallbackChainEntry> = Vec::new();
    assert!(get_fallback_model_group(&empty, "gpt-4", names).is_none());
    assert!(resolve_for_lookup_groups(&empty, &["gpt-4"], names).is_none());
}

fn unmatched_entries_stay_out_of_the_way(names: &ProviderNames<'_>) {
    let repeated = vec![
        group("gpt-4", vec![name("first")]),
        group("gpt-4", vec![name("second")]),
    ];
    assert_eq!(
        matched_models(get_fallback_model_group(&repeated, "openai/gpt-4", names)),
        ["second"]
    );
    let unmatched = vec![group("gpt-4", vec![name("claude")])];
    assert!(get_fallback_model_group(&unmatched, "openai/other", names).is_none());
    assert!(get_fallback_model_group(&unmatched, "gpt-3.5", names).is_none());

    let wildcard = vec![group("*", vec![name("backup")])];
    let stripped_wildcard = get_fallback_model_group(&wildcard, "openai/*", names).unwrap();
    assert!(!stripped_wildcard.generic);
    let generic = get_fallback_model_group(&wildcard, "anything", names).unwrap();
    assert!(generic.generic);
}

fn lookup_group_selection() {
    let mut state = request_state(bucket(None, None, None), Bucket::default());
    assert!(fallback_lookup_groups(&state, None).is_empty());
    state.metadata = bucket(Some("tier"), Some("routed"), Some("original"));
    assert_eq!(
        fallback_lookup_groups(&state, Some("requested")),
        ["tier", "routed", "requested", "original"]
    );
    assert_eq!(
        fallback_lookup_groups(&state, Some("tier")),
        ["tier", "routed", "original"]
    );
    let mut explicit_empty = request_state(bucket(Some(""), None, None), Bucket::default());
    explicit_empty.metadata.values.model_group = Some(String::new());
    assert!(fallback_lookup_groups(&explicit_empty, Some("")).is_empty());
}

fn depth_bounds() {
    assert_eq!(
        advance_depth(FallbackBudget {
            depth: 0,
            max_fallbacks: 0,
        }),
        DepthOutcome::Exhausted
    );
    assert_eq!(
        advance_depth(FallbackBudget {
            depth: 4,
            max_fallbacks: 5,
        }),
        DepthOutcome::Continue { next_depth: 5 }
    );
}

#[test]
fn canonical_targets_keep_distinct_parameters() {
    let first = parameterized(
        "claude",
        &[
            ("a", json!(1_i64)),
            ("b", json!(2_i64)),
            ("list", json!([1_i64, 2_i64])),
        ],
    );
    let reordered = parameterized(
        "claude",
        &[
            ("list", json!([1_i64, 2_i64])),
            ("b", json!(2_i64)),
            ("a", json!(1_i64)),
        ],
    );
    let colliding = parameterized(
        "claude",
        &[
            ("a", json!(1_i64)),
            ("b", json!(2_i64)),
            ("list", json!([12_i64])),
        ],
    );
    let recorded = AttemptedTargets::from_targets(&[first]);
    assert!(recorded.keys.contains(&attempt_key(&reordered)));
    assert!(!recorded.keys.contains(&attempt_key(&colliding)));
}

fn request_chains_resolve(names: &ProviderNames<'_>) {
    let settings = Settings {
        fallbacks: vec![group("gpt-4", vec![name("router")])],
        context_window_fallbacks: vec![group("gpt-4", vec![name("router-long")])],
        content_policy_fallbacks: Vec::new(),
        max_fallbacks: 5,
    };
    let override_chain = vec![group("gpt-4", vec![name("request")])];
    let overrides = RequestOverrides {
        fallbacks: Some(&override_chain),
        context_window_fallbacks: None,
        content_policy_fallbacks: None,
        num_retries: None,
        model_group_retry_policy: None,
        depth: 2,
    };
    let selected = select_chains(&settings, &overrides);
    assert_eq!(selected.budget.depth, 2);
    assert_eq!(selected.budget.max_fallbacks, 5);
    let attempted = AttemptedTargets::default();
    let resolved = resolve_chain(selected.ordinary, &["gpt-4"], names, &attempted).unwrap();
    assert_eq!(target_models(&resolved.targets), ["request"]);
    assert!(resolved.has_unattempted);
    let window = resolve_chain(selected.context_window, &["gpt-4"], names, &attempted).unwrap();
    assert_eq!(target_models(&window.targets), ["router-long"]);
    empty_request_chain_disables(&settings, names);
    let tried = AttemptedTargets::from_targets(&[name("request")]);
    let retried = resolve_chain(selected.ordinary, &["gpt-4"], names, &tried).unwrap();
    assert!(!retried.has_unattempted);
}

fn empty_request_chain_disables(settings: &Settings, names: &ProviderNames<'_>) {
    let disabled = select_chains(
        settings,
        &RequestOverrides {
            fallbacks: Some(&[]),
            context_window_fallbacks: None,
            content_policy_fallbacks: None,
            num_retries: None,
            model_group_retry_policy: None,
            depth: 0,
        },
    );
    let attempted = AttemptedTargets::default();
    assert!(resolve_chain(disabled.ordinary, &["gpt-4"], names, &attempted).is_none());
}

fn selection_and_pre_routing() {
    let mut state = request_state(
        bucket(Some("tier"), Some("requested"), Some("original")),
        Bucket::default(),
    );
    assert_eq!(get_pre_routing_selection(&state), Some("tier"));
    record_pre_routing_selection(&mut state, "other-tier");
    assert_eq!(get_pre_routing_selection(&state), Some("other-tier"));
    clear_pre_routing_selection(&mut state);
    assert_eq!(get_pre_routing_selection(&state), None);
    assert_eq!(
        fallback_lookup_groups(&state, Some("requested")),
        ["requested", "original"]
    );
    the_litellm_metadata_bucket_wins_when_present();
}

#[rstest]
#[case::bare_and_named_are_one_attempt(&[][..], &[FallbackTarget::Model(String::from("claude"))][..], true)]
#[case::a_different_order_is_a_different_attempt(
    &[("_target_order", json!(1_i64))][..],
    &[parameterized("claude", &[("_target_order", json!(2_i64))])][..],
    false
)]
#[case::the_same_parameters_are_one_attempt(
    &[("_target_order", json!(1_i64))][..],
    &[parameterized("claude", &[("_target_order", json!(1_i64))])][..],
    true
)]
fn attempt_identity_compares_canonical_targets(
    #[case] parameters: &[(&str, JsonValue)],
    #[case] attempted: &[FallbackTarget],
    #[case] same: bool,
) {
    let target = parameterized("claude", parameters);
    let recorded = AttemptedTargets::from_targets(attempted);
    let key = attempt_key(&target);
    assert_eq!(recorded.keys.contains(&key), same);
    assert_eq!(key == attempt_key(&attempted[0]), same);
}

#[test]
fn has_unattempted_counts_every_target() {
    let chain = vec![name("one"), name("two")];
    let partial = AttemptedTargets::from_targets(&[name("one")]);
    let refs: Vec<&FallbackTarget> = chain.iter().collect();
    assert!(has_unattempted_targets(refs.as_slice(), &partial));
    let complete = AttemptedTargets::from_targets(&chain);
    assert!(!has_unattempted_targets(refs.as_slice(), &complete));
    let empty: [&FallbackTarget; 0] = [];
    assert!(!has_unattempted_targets(&empty, &partial));
}

#[test]
fn the_litellm_metadata_bucket_wins_when_present() {
    let mut state = request_state(
        bucket(Some("metadata-tier"), None, None),
        bucket(Some("litellm-tier"), None, None),
    );
    assert_eq!(get_pre_routing_selection(&state), Some("metadata-tier"));
    record_pre_routing_selection(&mut state, "selected");
    assert_eq!(
        state
            .litellm_metadata
            .values
            .pre_routing_selected_model
            .as_deref(),
        Some("selected")
    );
    assert_eq!(
        state.metadata.values.pre_routing_selected_model.as_deref(),
        Some("metadata-tier")
    );
    let mut snapshot = state.clone();
    carry_pre_routing_selection(&state, &mut snapshot);
    assert_eq!(get_pre_routing_selection(&snapshot), Some("metadata-tier"));
    clear_pre_routing_selection(&mut snapshot);
    assert_eq!(get_pre_routing_selection(&snapshot), None);
    let live = request_state(Bucket::default(), bucket(Some("live-tier"), None, None));
    let mut carried = request_state(Bucket::default(), bucket(Some("old-tier"), None, None));
    carry_pre_routing_selection(&live, &mut carried);
    assert_eq!(get_pre_routing_selection(&carried), Some("live-tier"));
}

#[test]
fn an_absent_bucket_takes_no_writes() {
    let mut state = request_state(Bucket::default(), Bucket::default());
    record_pre_routing_selection(&mut state, "tier");
    record_disable_fallbacks(&mut state, true);
    assert_eq!(state, RequestMetadata::default());
}

#[test]
fn disabling_fallbacks_is_read_from_the_raw_value_or_either_bucket() {
    let mut state = request_state(bucket(None, None, None), Bucket::default());
    assert!(!fallbacks_disabled_for_request(&state, false));
    assert!(fallbacks_disabled_for_request(&state, true));
    record_disable_fallbacks(&mut state, true);
    assert!(fallbacks_disabled_for_request(&state, false));
    assert!(state.metadata.values.disable_fallbacks);
    record_disable_fallbacks(&mut state, false);
    assert!(!fallbacks_disabled_for_request(&state, false));
}

#[test]
fn the_hop_copies_the_metadata_and_keeps_the_controls() {
    let carried = bucket(Some("tier"), None, None);
    let mut absent = bucket(Some("stale"), None, None);
    absent.present = false;
    let mut live = request_state(carried.clone(), absent);
    let request = vec![group("gpt-4", vec![name("request")])];
    let empty: Vec<FallbackChainEntry> = Vec::new();
    let policy: JsonValue =
        serde_json::from_str(r#"{"gpt-4":{"RateLimitErrorRetries":3}}"#).unwrap();
    let overrides = RequestOverrides {
        fallbacks: Some(&request),
        context_window_fallbacks: None,
        content_policy_fallbacks: Some(&empty),
        num_retries: Some(0),
        model_group_retry_policy: Some(&policy),
        depth: 1,
    };
    let hop = HopRequest::for_hop("gpt-4", overrides, &live);
    record_pre_routing_selection(&mut live, "changed");
    assert_eq!(hop.model, "gpt-4");
    assert_eq!(hop.overrides, overrides);
    assert_eq!(hop.overrides.fallbacks, Some(request.as_slice()));
    assert_eq!(hop.overrides.num_retries, Some(0));
    assert_eq!(hop.overrides.model_group_retry_policy, Some(&policy));
    assert_eq!(hop.metadata, request_state(carried, Bucket::default()));
    assert_eq!(get_pre_routing_selection(&hop.metadata), Some("tier"));
    assert_eq!(get_pre_routing_selection(&live), Some("changed"));
}

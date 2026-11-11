#![cfg(test)]

use betterlite_config::sections::fallback_plans::chains::{
    FallbackChainEntry, FallbackTarget, ParameterizedTarget,
};
use betterlite_config::sections::settings::JsonValue;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::json;

use crate::fallback_plans::attempts::{AttemptedTargets, attempt_key, has_unattempted_targets};
use crate::fallback_plans::chains::{
    ChainMatch, ProviderNames, get_fallback_model_group, resolve_for_lookup_groups,
};
use crate::fallback_plans::controls::{mid_stream_hop, per_request_fallback_controls};
use crate::fallback_plans::groups::{
    Bucket, MetadataValues, RequestMetadata, carry_pre_routing_selection,
    clear_pre_routing_selection, fallback_lookup_groups, fallbacks_disabled_for_request,
    get_pre_routing_selection, record_disable_fallbacks, record_pre_routing_selection,
};
use crate::fallback_plans::plan::{
    DepthOutcome, FallbackBudget, FallbackPlans, RequestOverrides, advance_depth, resolve_chain,
};

const SLUGS: [&str; 2] = ["openai", "anthropic"];

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

fn no_inference(_model: &str) -> Option<String> {
    None
}

fn openai_inference(model: &str) -> Option<String> {
    model.starts_with("gpt").then(|| String::from("openai"))
}

fn slugs() -> Vec<String> {
    SLUGS.iter().map(|slug| (*slug).to_owned()).collect()
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
    let slugs = slugs();
    let plain = no_inference;
    let names = ProviderNames {
        slugs: &slugs,
        inferred: &plain,
    };
    let exact = get_fallback_model_group(&chains, "gpt-4", &names).unwrap();
    assert_eq!(target_models(&exact.targets), ["claude"]);
    assert!(!exact.generic);
    let generic = get_fallback_model_group(&chains, "unknown", &names).unwrap();
    assert_eq!(target_models(&generic.targets), ["backup"]);
    assert!(generic.generic);
    lookup_names_resolve(&slugs);
    request_chains_resolve(&slugs);
    selection_and_pre_routing();
}

fn lookup_names_resolve(slugs: &[String]) {
    let plain = no_inference;
    let names = ProviderNames {
        slugs,
        inferred: &plain,
    };
    let stripped = vec![group("gpt-4", vec![name("claude")])];
    assert_eq!(
        matched_models(get_fallback_model_group(&stripped, "openai/gpt-4", &names)),
        ["claude"]
    );
    let inferred = openai_inference;
    let naming = ProviderNames {
        slugs,
        inferred: &inferred,
    };
    let prefixed = vec![group("openai/gpt-4", vec![name("claude")])];
    assert_eq!(
        matched_models(get_fallback_model_group(&prefixed, "gpt-4", &naming)),
        ["claude"]
    );
    let bare = vec![FallbackChainEntry::Target(name("claude"))];
    assert_eq!(
        matched_models(get_fallback_model_group(&bare, "anything", &names)),
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
            &names
        )),
        ["claude"]
    );
}

#[test]
fn fallback_plans_boundaries() {
    let slugs = slugs();
    let plain = no_inference;
    let names = ProviderNames {
        slugs: &slugs,
        inferred: &plain,
    };
    let empty: Vec<FallbackChainEntry> = Vec::new();
    assert!(get_fallback_model_group(&empty, "gpt-4", &names).is_none());
    assert!(resolve_for_lookup_groups(&empty, &["gpt-4"], &names).is_none());

    let repeated = vec![
        group("gpt-4", vec![name("first")]),
        group("gpt-4", vec![name("second")]),
    ];
    assert_eq!(
        matched_models(get_fallback_model_group(&repeated, "openai/gpt-4", &names)),
        ["second"]
    );

    let wildcard = vec![group("*", vec![name("backup")])];
    let stripped_wildcard = get_fallback_model_group(&wildcard, "openai/*", &names).unwrap();
    assert!(!stripped_wildcard.generic);
    let generic = get_fallback_model_group(&wildcard, "anything", &names).unwrap();
    assert!(generic.generic);

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
}

fn request_chains_resolve(slugs: &[String]) {
    let plain = no_inference;
    let names = ProviderNames {
        slugs,
        inferred: &plain,
    };
    let plans = FallbackPlans {
        fallbacks: vec![group("gpt-4", vec![name("router")])],
        context_window_fallbacks: Vec::new(),
        content_policy_fallbacks: Vec::new(),
        max_fallbacks: 5,
    };
    let override_chain = vec![group("gpt-4", vec![name("request")])];
    let overrides = RequestOverrides {
        fallbacks: Some(&override_chain),
        context_window_fallbacks: None,
        content_policy_fallbacks: None,
        depth: 2,
    };
    let selected = plans.chains(&overrides);
    assert_eq!(selected.budget.depth, 2);
    assert_eq!(selected.budget.max_fallbacks, 5);
    let attempted = AttemptedTargets::default();
    let resolved = resolve_chain(selected.ordinary, &["gpt-4"], &names, &attempted).unwrap();
    assert_eq!(target_models(&resolved.targets), ["request"]);
    assert!(resolved.has_unattempted);
    let disabled = plans.chains(&RequestOverrides {
        fallbacks: Some(&[]),
        context_window_fallbacks: None,
        content_policy_fallbacks: None,
        depth: 0,
    });
    assert!(resolve_chain(disabled.ordinary, &["gpt-4"], &names, &attempted).is_none());
    let tried = AttemptedTargets::from_targets(&[name("request")]);
    let retried = resolve_chain(selected.ordinary, &["gpt-4"], &names, &tried).unwrap();
    assert!(!retried.has_unattempted);
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
    assert_eq!(recorded.contains(&target), same);
    assert!(attempt_key(&target).is_some());
}

#[test]
fn has_unattempted_counts_every_target() {
    let chain = vec![name("one"), name("two")];
    let partial = AttemptedTargets::from_targets(&[name("one")]);
    let refs: Vec<&FallbackTarget> = chain.iter().collect();
    assert!(has_unattempted_targets(Some(refs.as_slice()), &partial));
    let complete = AttemptedTargets::from_targets(&chain);
    assert!(!has_unattempted_targets(Some(refs.as_slice()), &complete));
    assert!(!has_unattempted_targets(None, &partial));
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
    let mut live = request_state(bucket(Some("tier"), None, None), Bucket::default());
    let request = vec![group("gpt-4", vec![name("request")])];
    let empty: Vec<FallbackChainEntry> = Vec::new();
    let overrides = RequestOverrides {
        fallbacks: Some(&request),
        context_window_fallbacks: None,
        content_policy_fallbacks: Some(&empty),
        depth: 1,
    };
    let budget = FallbackBudget {
        depth: 1,
        max_fallbacks: 5,
    };
    let hop = mid_stream_hop(
        "gpt-4",
        per_request_fallback_controls(&overrides),
        &live,
        budget,
    );
    assert_eq!(hop.model, "gpt-4");
    assert_eq!(hop.controls.fallbacks, Some(request.as_slice()));
    assert_eq!(hop.controls.context_window_fallbacks, None);
    assert!(
        hop.controls
            .content_policy_fallbacks
            .is_some_and(<[FallbackChainEntry]>::is_empty)
    );
    assert_eq!(hop.budget, budget);
    assert_eq!(hop.metadata, live);
    record_pre_routing_selection(&mut live, "changed");
    assert_eq!(get_pre_routing_selection(&hop.metadata), Some("tier"));
}

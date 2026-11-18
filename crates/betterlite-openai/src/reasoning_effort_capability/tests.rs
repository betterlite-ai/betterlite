#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use indexmap::IndexMap;
use rstest::rstest;

use super::declared::{
    ADVERTISEMENT_ORDER, EffortFlag, declared_efforts_for_model, effort_from_wire, effort_wire_name,
};
use super::{
    GroupReasoningEfforts, deployment_is_catalog_mapped, intersect_supported_efforts,
    nearest_declared_effort, resolve_supported_efforts, supports_none_reasoning_effort,
};

fn declared_levels_win_over_flags() {
    let map = ModelCostMap::parse(b"{}").unwrap();
    let model_info: IndexMap<String, Json> = serde_json::from_str(
        r#"{"supports_reasoning": true, "supports_xhigh_reasoning_effort": true, "reasoning_effort_levels": ["max", "low", "bogus"]}"#,
    )
    .unwrap();
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(vec![ReasoningEffort::Low, ReasoningEffort::Max])
    );
}

fn flags_advertise_the_polarity_of_each_level() {
    let map = ModelCostMap::parse(b"{}").unwrap();
    let model_info: IndexMap<String, Json> = serde_json::from_str(
        r#"{"supports_reasoning": true, "supports_low_reasoning_effort": false, "supports_xhigh_reasoning_effort": true}"#,
    )
    .unwrap();
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(vec![
            ReasoningEffort::None,
            ReasoningEffort::Minimal,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
        ])
    );
}

fn a_mapped_model_without_flags_is_known_empty() {
    let map = ModelCostMap::parse(b"{}").unwrap();
    let key_only: IndexMap<String, Json> = serde_json::from_str(r#"{"key": "openai/x"}"#).unwrap();
    assert_eq!(
        resolve_supported_efforts(&key_only, true, &map),
        Some(Vec::new())
    );
    assert_eq!(resolve_supported_efforts(&key_only, false, &map), None);
    let reasoning: IndexMap<String, Json> =
        serde_json::from_str(r#"{"key": "openai/x", "supports_reasoning": true}"#).unwrap();
    assert_eq!(resolve_supported_efforts(&reasoning, true, &map), None);
}

fn a_per_level_flag_alone_proves_reasoning() {
    let map = ModelCostMap::parse(b"{}").unwrap();
    let model_info: IndexMap<String, Json> =
        serde_json::from_str(r#"{"supports_minimal_reasoning_effort": "yes"}"#).unwrap();
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(vec![
            ReasoningEffort::None,
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ])
    );
}

fn explicit_false_empties_even_with_levels() {
    let map = ModelCostMap::parse(b"{}").unwrap();
    let model_info: IndexMap<String, Json> = serde_json::from_str(
        r#"{"supports_reasoning": false, "supports_low_reasoning_effort": true}"#,
    )
    .unwrap();
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(Vec::new())
    );
}

fn the_own_flag_wins_over_the_bare_twin() {
    let bare_only =
        ModelCostMap::parse(br#"{"gpt-5-mini": {"supports_low_reasoning_effort": false}}"#)
            .unwrap();
    let bare_info: IndexMap<String, Json> = serde_json::from_str(
        r#"{"key": "azure/gpt-5-mini", "litellm_provider": "azure", "supports_reasoning": true}"#,
    )
    .unwrap();
    assert_eq!(
        resolve_supported_efforts(&bare_info, false, &bare_only),
        Some(vec![
            ReasoningEffort::Minimal,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ])
    );
    let map = ModelCostMap::parse(
        br#"{"gpt-5-mini": {"supports_low_reasoning_effort": false, "supports_xhigh_reasoning_effort": true}}"#,
    )
    .unwrap();
    let model_info: IndexMap<String, Json> = serde_json::from_str(
        r#"{"key": "azure/gpt-5-mini", "litellm_provider": "azure", "supports_reasoning": true, "supports_low_reasoning_effort": true, "supports_xhigh_reasoning_effort": false}"#,
    )
    .unwrap();
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(vec![
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ])
    );
}

fn declared_levels_read_the_prefixed_entry_then_the_bare_one() {
    let map = ModelCostMap::parse(
        br#"{
            "openai/m": {"reasoning_effort_levels": ["high"]},
            "m": {"reasoning_effort_levels": ["low"]},
            "openai/e": {},
            "e": {"reasoning_effort_levels": ["max"]},
            "openai/s": "nope",
            "s": {"reasoning_effort_levels": ["low"]},
            "openai/n": null,
            "n": {"reasoning_effort_levels": ["minimal"]}
        }"#,
    )
    .unwrap();
    assert_eq!(
        declared_efforts_for_model("m", "openai", &map),
        Some(vec![ReasoningEffort::High])
    );
    assert_eq!(
        declared_efforts_for_model("e", "openai", &map),
        Some(vec![ReasoningEffort::Max])
    );
    assert_eq!(
        declared_efforts_for_model("n", "openai", &map),
        Some(vec![ReasoningEffort::Minimal])
    );
    assert_eq!(declared_efforts_for_model("s", "openai", &map), None);
    assert_eq!(declared_efforts_for_model("absent", "openai", &map), None);
}

fn a_prefixed_model_declares_the_bare_entry_levels() {
    let map =
        ModelCostMap::parse(br#"{"m": {"reasoning_effort_levels": ["high", "low"]}}"#).unwrap();
    let model_info: IndexMap<String, Json> = serde_json::from_str(
        r#"{"key": "openai/m", "litellm_provider": "openai", "supports_reasoning": true}"#,
    )
    .unwrap();
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(vec![ReasoningEffort::Low, ReasoningEffort::High])
    );
}

fn intersection_keeps_the_advertisement_order() {
    assert_eq!(
        intersect_supported_efforts(
            Some(&[ReasoningEffort::High, ReasoningEffort::Low]),
            Some(&[ReasoningEffort::Low, ReasoningEffort::High]),
        ),
        Some(vec![ReasoningEffort::Low, ReasoningEffort::High])
    );
    assert_eq!(
        intersect_supported_efforts(
            Some(&[ReasoningEffort::High, ReasoningEffort::None]),
            Some(&[ReasoningEffort::None, ReasoningEffort::High]),
        ),
        Some(vec![ReasoningEffort::None, ReasoningEffort::High])
    );
}

fn an_unknown_side_never_narrows() {
    assert_eq!(
        intersect_supported_efforts(Some(&[ReasoningEffort::Low]), None),
        Some(vec![ReasoningEffort::Low])
    );
    assert_eq!(
        intersect_supported_efforts(None, Some(&[ReasoningEffort::Max])),
        Some(vec![ReasoningEffort::Max])
    );
    assert_eq!(intersect_supported_efforts(None, None), None);
}

fn the_group_aggregate_intersects_known_deployments() {
    let mut efforts = GroupReasoningEfforts::default();
    efforts.observe(Some(vec![ReasoningEffort::Low, ReasoningEffort::High]));
    efforts.observe(Some(vec![ReasoningEffort::High]));
    assert_eq!(efforts.supported, Some(vec![ReasoningEffort::High]));
    assert!(!efforts.unknown);
}

fn the_group_aggregate_clears_on_an_unknown_deployment() {
    let mut efforts = GroupReasoningEfforts::default();
    efforts.observe(None);
    efforts.observe(Some(vec![ReasoningEffort::Low]));
    assert_eq!(efforts.supported, None);
    assert!(efforts.unknown);
}

fn catalog_mapping_needs_both_modes() {
    let mode: IndexMap<String, Json> = serde_json::from_str(r#"{"mode": "chat"}"#).unwrap();
    let empty = IndexMap::new();
    assert!(deployment_is_catalog_mapped(Some(&mode), &empty));
    assert!(!deployment_is_catalog_mapped(None, &empty));
    assert!(!deployment_is_catalog_mapped(Some(&empty), &empty));
    assert!(!deployment_is_catalog_mapped(Some(&mode), &mode));
}

fn nearest_rounds_up_to_the_weakest_declared_level() {
    assert_eq!(
        nearest_declared_effort(
            ReasoningEffort::Low,
            &[ReasoningEffort::High, ReasoningEffort::Minimal],
        ),
        ReasoningEffort::High
    );
    assert_eq!(
        nearest_declared_effort(
            ReasoningEffort::Xhigh,
            &[ReasoningEffort::High, ReasoningEffort::Max],
        ),
        ReasoningEffort::Max
    );
}

fn nearest_rounds_down_above_every_level() {
    assert_eq!(
        nearest_declared_effort(
            ReasoningEffort::Max,
            &[ReasoningEffort::Minimal, ReasoningEffort::Low],
        ),
        ReasoningEffort::Low
    );
}

fn nearest_leaves_a_declared_level_and_none_alone() {
    assert_eq!(
        nearest_declared_effort(
            ReasoningEffort::Low,
            &[ReasoningEffort::None, ReasoningEffort::Low],
        ),
        ReasoningEffort::Low
    );
    assert_eq!(
        nearest_declared_effort(ReasoningEffort::None, &[ReasoningEffort::Low]),
        ReasoningEffort::None
    );
}

fn the_none_flag_follows_the_provider_family() {
    let gpt: IndexMap<String, Json> =
        serde_json::from_str(r#"{"key": "azure/gpt-5", "litellm_provider": "azure"}"#).unwrap();
    assert!(!supports_none_reasoning_effort(&gpt, EffortFlag::Absent));
    assert!(supports_none_reasoning_effort(&gpt, EffortFlag::True));
    assert!(!supports_none_reasoning_effort(&gpt, EffortFlag::False));
    let chat: IndexMap<String, Json> =
        serde_json::from_str(r#"{"key": "azure/gpt-5-chat", "litellm_provider": "azure"}"#)
            .unwrap();
    assert!(supports_none_reasoning_effort(&chat, EffortFlag::Absent));
    let other: IndexMap<String, Json> =
        serde_json::from_str(r#"{"key": "openai/gpt-5", "litellm_provider": "openai"}"#).unwrap();
    assert!(supports_none_reasoning_effort(&other, EffortFlag::Absent));
    assert!(!supports_none_reasoning_effort(&other, EffortFlag::False));
    let unnamed: IndexMap<String, Json> =
        serde_json::from_str(r#"{"litellm_provider": "azure"}"#).unwrap();
    assert!(supports_none_reasoning_effort(&unnamed, EffortFlag::Absent));
}

fn effort_wire_names_round_trip() {
    for effort in ADVERTISEMENT_ORDER {
        assert_eq!(effort_from_wire(effort_wire_name(effort)), Some(effort));
    }
    assert_eq!(effort_from_wire("bogus"), None);
}

fn an_empty_declared_list_keeps_the_request() {
    assert_eq!(
        nearest_declared_effort(ReasoningEffort::Minimal, &[]),
        ReasoningEffort::Minimal
    );
}

fn an_empty_side_empties_the_intersection() {
    assert_eq!(
        intersect_supported_efforts(Some(&[]), Some(&[ReasoningEffort::Low])),
        Some(Vec::new())
    );
    assert_eq!(
        intersect_supported_efforts(Some(&[ReasoningEffort::Low]), Some(&[])),
        Some(Vec::new())
    );
}

fn a_late_unknown_deployment_clears_the_aggregate() {
    let mut efforts = GroupReasoningEfforts::default();
    efforts.observe(Some(vec![ReasoningEffort::Low]));
    efforts.observe(None);
    assert_eq!(efforts.supported, None);
    assert!(efforts.unknown);
}

fn an_empty_declared_list_is_a_known_empty() {
    let map = ModelCostMap::parse(b"{}").unwrap();
    let model_info: IndexMap<String, Json> =
        serde_json::from_str(r#"{"supports_reasoning": true, "reasoning_effort_levels": []}"#)
            .unwrap();
    assert_eq!(
        resolve_supported_efforts(&model_info, true, &map),
        Some(Vec::new())
    );
}

fn a_declared_value_that_is_not_a_list_falls_back_to_the_flags() {
    let map = ModelCostMap::parse(b"{}").unwrap();
    let model_info: IndexMap<String, Json> =
        serde_json::from_str(r#"{"supports_reasoning": true, "reasoning_effort_levels": "high"}"#)
            .unwrap();
    assert_eq!(resolve_supported_efforts(&model_info, false, &map), None);
}

fn a_flag_that_is_not_a_boolean_still_advertises() {
    let map = ModelCostMap::parse(b"{}").unwrap();
    let model_info: IndexMap<String, Json> = serde_json::from_str(
        r#"{"supports_reasoning": true, "supports_minimal_reasoning_effort": "yes"}"#,
    )
    .unwrap();
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(vec![
            ReasoningEffort::None,
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ])
    );
}

#[rstest]
#[case::declared_levels_win_over_flags(declared_levels_win_over_flags)]
#[case::flags_advertise_the_polarity_of_each_level(flags_advertise_the_polarity_of_each_level)]
#[case::a_mapped_model_without_flags_is_known_empty(a_mapped_model_without_flags_is_known_empty)]
#[case::a_per_level_flag_alone_proves_reasoning(a_per_level_flag_alone_proves_reasoning)]
#[case::explicit_false_empties_even_with_levels(explicit_false_empties_even_with_levels)]
#[case::the_own_flag_wins_over_the_bare_twin(the_own_flag_wins_over_the_bare_twin)]
#[case::declared_levels_read_the_prefixed_entry_then_the_bare_one(
    declared_levels_read_the_prefixed_entry_then_the_bare_one
)]
#[case::a_prefixed_model_declares_the_bare_entry_levels(
    a_prefixed_model_declares_the_bare_entry_levels
)]
#[case::intersection_keeps_the_advertisement_order(intersection_keeps_the_advertisement_order)]
#[case::an_unknown_side_never_narrows(an_unknown_side_never_narrows)]
#[case::the_group_aggregate_intersects_known_deployments(
    the_group_aggregate_intersects_known_deployments
)]
#[case::the_group_aggregate_clears_on_an_unknown_deployment(
    the_group_aggregate_clears_on_an_unknown_deployment
)]
#[case::catalog_mapping_needs_both_modes(catalog_mapping_needs_both_modes)]
#[case::nearest_rounds_up_to_the_weakest_declared_level(
    nearest_rounds_up_to_the_weakest_declared_level
)]
#[case::nearest_rounds_down_above_every_level(nearest_rounds_down_above_every_level)]
#[case::nearest_leaves_a_declared_level_and_none_alone(
    nearest_leaves_a_declared_level_and_none_alone
)]
#[case::the_none_flag_follows_the_provider_family(the_none_flag_follows_the_provider_family)]
#[case::effort_wire_names_round_trip(effort_wire_names_round_trip)]
fn reasoning_effort_capability_behaviors(#[case] case: fn()) {
    case();
}

#[rstest]
#[case::an_empty_declared_list_keeps_the_request(an_empty_declared_list_keeps_the_request)]
#[case::an_empty_side_empties_the_intersection(an_empty_side_empties_the_intersection)]
#[case::a_late_unknown_deployment_clears_the_aggregate(
    a_late_unknown_deployment_clears_the_aggregate
)]
#[case::an_empty_declared_list_is_a_known_empty(an_empty_declared_list_is_a_known_empty)]
#[case::a_declared_value_that_is_not_a_list_falls_back_to_the_flags(
    a_declared_value_that_is_not_a_list_falls_back_to_the_flags
)]
#[case::a_flag_that_is_not_a_boolean_still_advertises(
    a_flag_that_is_not_a_boolean_still_advertises
)]
fn reasoning_effort_capability_boundaries(#[case] case: fn()) {
    case();
}

use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;

use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::context_placement::ContextPlacement;
use crate::context_placement::facts::{
    CatalogModelInfo, ModelLimitKey, WindowFacts, deployment_limit,
};
use crate::context_placement::fit::ContextFit;
use crate::context_placement::placement::{Placement, PlacementRequest, apply_context_placement};
use crate::context_placement::request::{
    OutOfBand, RequestFacts, out_of_band_text, request_byte_upper_bound,
};
use crate::tier_definitions::model::TierSet;

use super::support::{
    ExtraModelInfo, FixedCounter, RecordingCounter, catalog, config, deployment, json, message,
    model_catalog, service, tier_set, with_router,
};

fn fit(service: &ContextPlacement<'_>, set: &TierSet, request: &RequestFacts<'_>) -> ContextFit {
    super::runtime().block_on(service.request_context_fit(set, None, request))
}

fn place(service: &ContextPlacement<'_>, input: &PlacementRequest<'_>) -> Option<Placement> {
    super::runtime()
        .block_on(service.context_window_placement(input, None))
        .unwrap()
}

fn window(input: u64) -> String {
    format!(r#"{{"max_input_tokens":{input}}}"#)
}

pub(super) fn run(name: &str) {
    match name {
        "disabled_gate_skips_the_fit_and_placement" => {
            disabled_gate_skips_the_fit_and_placement();
        }
        "compaction_pending_skips_the_fit_and_placement" => {
            compaction_pending_skips_the_fit_and_placement();
        }
        "no_messages_skips_the_fit_and_placement" => no_messages_skips_the_fit_and_placement(),
        "a_full_pool_keeps_the_tier" => a_full_pool_keeps_the_tier(),
        "a_partial_pool_restricts_the_allowed_models" => {
            a_partial_pool_restricts_the_allowed_models();
        }
        "escalation_moves_to_the_first_provable_tier" => {
            escalation_moves_to_the_first_provable_tier();
        }
        "escalation_skips_a_tier_without_proven_models" => {
            escalation_skips_a_tier_without_proven_models();
        }
        "a_group_with_an_unknown_member_is_not_escalated_onto" => {
            a_group_with_an_unknown_member_is_not_escalated_onto();
        }
        "unknown_windows_are_holdable_but_not_provable" => {
            unknown_windows_are_holdable_but_not_provable();
        }
        "a_pool_override_replaces_the_tier_pool" => a_pool_override_replaces_the_tier_pool(),
        "an_empty_pool_returns_no_placement" => an_empty_pool_returns_no_placement(),
        "a_named_tier_can_hold_a_partial_pool" => a_named_tier_can_hold_a_partial_pool(),
        "a_byte_bound_under_the_window_skips_the_count" => {
            a_byte_bound_under_the_window_skips_the_count();
        }
        "a_byte_bound_over_the_window_counts" => a_byte_bound_over_the_window_counts(),
        "the_count_covers_the_out_of_band_text" => the_count_covers_the_out_of_band_text(),
        "out_of_band_joins_instructions_and_system" => out_of_band_joins_instructions_and_system(),
        "body_tools_win_over_request_tools" => body_tools_win_over_request_tools(),
        "empty_body_tools_fall_back_to_request_tools" => {
            empty_body_tools_fall_back_to_request_tools();
        }
        "a_null_or_non_text_value_contributes_nothing" => {
            a_null_or_non_text_value_contributes_nothing();
        }
        "the_output_ceiling_is_the_smallest_limit" => the_output_ceiling_is_the_smallest_limit(),
        "an_undeclared_output_limit_removes_the_ceiling" => {
            an_undeclared_output_limit_removes_the_ceiling();
        }
        "a_declared_limit_beats_the_router_info" => a_declared_limit_beats_the_router_info(),
        "an_authenticating_provider_has_no_router_limit" => {
            an_authenticating_provider_has_no_router_limit();
        }
        "a_custom_provider_overrides_the_model_prefix" => {
            a_custom_provider_overrides_the_model_prefix();
        }
        "catalog_limits_resolve_for_a_declared_model" => {
            catalog_limits_resolve_for_a_declared_model();
        }
        "catalog_azure_resolves_by_base_model" => catalog_azure_resolves_by_base_model(),
        "catalog_unmapped_models_have_no_limit" => catalog_unmapped_models_have_no_limit(),
        "message_without_content_adds_no_bytes" => message_without_content_adds_no_bytes(),
        "message_parts_measure_as_their_json" => message_parts_measure_as_their_json(),
        "a_match_by_name_returns_the_placement_tier" => {
            a_match_by_name_returns_the_placement_tier()
        }
        "no_placement_leaves_the_tier_and_signals" => no_placement_leaves_the_tier_and_signals(),
        "the_fit_accepts_an_unknown_group" => the_fit_accepts_an_unknown_group(),
        "a_disabled_gate_ignores_a_supplied_fit" => a_disabled_gate_ignores_a_supplied_fit(),
        "catalog_custom_provider_overrides_the_model_prefix" => {
            catalog_custom_provider_overrides_the_model_prefix();
        }
        "an_empty_parts_message_adds_no_bytes" => an_empty_parts_message_adds_no_bytes(),
        "the_tool_truthiness_follows_python_values" => the_tool_truthiness_follows_python_values(),
        _ => panic!("unknown behavior case {name}"),
    }
}

fn disabled_gate_skips_the_fit_and_placement() {
    let config = config(false, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = RecordingCounter::new();
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(400))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let computed = fit(&placement_service, &set, &request);
    assert!(computed.facts.is_empty());
    assert_eq!(computed.needed, None);
    assert_eq!(computed.buffer.to_bits(), 0.95_f64.to_bits());
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input), None);
    assert!(counter.seen().is_empty());
}

fn compaction_pending_skips_the_fit_and_placement() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = RecordingCounter::new();
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(400))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: true,
    };
    assert_eq!(fit(&placement_service, &set, &request).needed, None);
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input), None);
    assert!(counter.seen().is_empty());
}

fn no_messages_skips_the_fit_and_placement() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 400 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages: [betterlite_openai::chat::ChatMessage; 0] = [];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    assert_eq!(fit(&placement_service, &set, &request).needed, None);
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input), None);
}

fn a_full_pool_keeps_the_tier() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 50 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let computed = fit(&placement_service, &set, &request);
    assert_eq!(computed.needed, Some(50));
    assert_eq!(
        computed.facts,
        BTreeMap::from([(
            "small".to_owned(),
            WindowFacts {
                window: Some(100),
                has_unknown: false,
            },
        )])
    );
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input), None);
}

fn a_partial_pool_restricts_the_allowed_models() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("tiny", r#"{"model":"openai/gpt-4o-mini"}"#, &window(50)),
        deployment("large", r#"{"model":"openai/gpt-4o"}"#, &window(10_000)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 60 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small", "tiny"]), ("MEDIUM", &["large"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let placement = place(&placement_service, &input).unwrap();
    assert_eq!(placement.tier, Tier::BuiltIn(ComplexityTier::Simple));
    assert_eq!(placement.allowed_models, vec!["small".to_owned()]);
    assert_eq!(
        placement.holdable_models,
        BTreeSet::from(["large".to_owned(), "small".to_owned()])
    );
}

fn escalation_moves_to_the_first_provable_tier() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("large", r#"{"model":"openai/gpt-4o"}"#, &window(10_000)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 200 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"]), ("MEDIUM", &["large"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let placement = place(&placement_service, &input).unwrap();
    assert_eq!(placement.tier, Tier::BuiltIn(ComplexityTier::Medium));
    assert_eq!(placement.allowed_models, vec!["large".to_owned()]);
    assert_eq!(
        placement.holdable_models,
        BTreeSet::from(["large".to_owned()])
    );
    let applied = apply_context_placement(tier, vec!["keep".to_owned()], Some(&placement));
    assert_eq!(applied.tier, Tier::BuiltIn(ComplexityTier::Medium));
    assert_eq!(
        applied.signals,
        vec!["keep".to_owned(), "context_escalation".to_owned()]
    );
    assert_eq!(
        applied.original_tier,
        Some(Tier::BuiltIn(ComplexityTier::Simple))
    );
}

fn escalation_skips_a_tier_without_proven_models() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("mixed", r#"{"model":"openai/gpt-4o"}"#, &window(1000)),
        deployment("mixed", r#"{"model":"openai/unknown"}"#, "{}"),
        deployment("big", r#"{"model":"openai/gpt-4o"}"#, &window(10_000)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 200 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[
        ("SIMPLE", &["small"]),
        ("MEDIUM", &["mixed"]),
        ("COMPLEX", &["big"]),
    ]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let placement = place(&placement_service, &input).unwrap();
    assert_eq!(placement.tier, Tier::BuiltIn(ComplexityTier::Complex));
    assert_eq!(placement.allowed_models, vec!["big".to_owned()]);
}

fn a_group_with_an_unknown_member_is_not_escalated_onto() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("mixed", r#"{"model":"openai/gpt-4o"}"#, &window(10_000)),
        deployment("mixed", r#"{"model":"openai/unknown"}"#, "{}"),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 200 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"]), ("MEDIUM", &["mixed"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input), None);
}

fn unknown_windows_are_holdable_but_not_provable() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("tiny", r#"{"model":"openai/gpt-4o-mini"}"#, &window(50)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 60 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small", "tiny"]), ("MEDIUM", &["mystery"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let placement = place(&placement_service, &input).unwrap();
    assert_eq!(
        placement.holdable_models,
        BTreeSet::from(["mystery".to_owned(), "small".to_owned()])
    );
}

fn a_pool_override_replaces_the_tier_pool() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("tiny", r#"{"model":"openai/gpt-4o-mini"}"#, &window(50)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 60 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let overridden = ["small".to_owned(), "tiny".to_owned()];
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: Some(&overridden),
        request,
    };
    let placement = place(&placement_service, &input).unwrap();
    assert_eq!(
        placement.allowed_models,
        vec!["small".to_owned()],
        "the decided tier's own pool holds fully and would return no placement"
    );
    let plain = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &plain), None);
}

fn an_empty_pool_returns_no_placement() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 200 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let tier = Tier::Named("empty".to_owned());
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input), None);
}

fn a_named_tier_can_hold_a_partial_pool() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("tiny", r#"{"model":"openai/gpt-4o-mini"}"#, &window(50)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 60 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("custom", &["small", "tiny"])]);
    let tier = Tier::Named("custom".to_owned());
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let placement = place(&placement_service, &input).unwrap();
    assert_eq!(placement.tier, Tier::Named("custom".to_owned()));
}

fn a_byte_bound_under_the_window_skips_the_count() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = RecordingCounter::new();
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let messages = [message(&"x".repeat(40))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    assert_eq!(fit(&placement_service, &set, &request).needed, None);
    assert!(counter.seen().is_empty());
}

fn a_byte_bound_over_the_window_counts() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = RecordingCounter::new();
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let messages = [message(&"x".repeat(96))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    assert_eq!(fit(&placement_service, &set, &request).needed, Some(1));
    assert_eq!(counter.seen(), vec![(1, String::new())]);
}

fn the_count_covers_the_out_of_band_text() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = RecordingCounter::new();
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let messages = [message(&"x".repeat(96))];
    let instructions = Json::Text("hello".to_owned());
    let system = Json::Text("world".to_owned());
    let out_of_band = OutOfBand {
        instructions: Some(&instructions),
        system: Some(&system),
        body_tools: None,
        request_tools: None,
    };
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    assert_eq!(fit(&placement_service, &set, &request).needed, Some(1));
    assert_eq!(counter.seen(), vec![(1, "helloworld".to_owned())]);
}

fn out_of_band_joins_instructions_and_system() {
    let instructions = Json::Text("abc".to_owned());
    let system = Json::Text("de".to_owned());
    let out_of_band = OutOfBand {
        instructions: Some(&instructions),
        system: Some(&system),
        body_tools: None,
        request_tools: None,
    };
    assert_eq!(out_of_band_text(&out_of_band), "abcde");
}

fn body_tools_win_over_request_tools() {
    let body = json(r#"[{"a":1}]"#);
    let request = json(r#"[{"b":2}]"#);
    let out_of_band = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&body),
        request_tools: Some(&request),
    };
    assert_eq!(out_of_band_text(&out_of_band), r#"[{"a":1}]"#);
}

fn empty_body_tools_fall_back_to_request_tools() {
    let body = json("[]");
    let request = json(r#"[{"b":2}]"#);
    let out_of_band = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&body),
        request_tools: Some(&request),
    };
    assert_eq!(out_of_band_text(&out_of_band), r#"[{"b":2}]"#);
}

fn a_null_or_non_text_value_contributes_nothing() {
    let instructions = Json::Bool(true);
    let system = Json::Null;
    let out_of_band = OutOfBand {
        instructions: Some(&instructions),
        system: Some(&system),
        body_tools: None,
        request_tools: None,
    };
    assert_eq!(out_of_band_text(&out_of_band), "");
}

fn the_output_ceiling_is_the_smallest_limit() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        with_router(
            deployment("group", r#"{"model":"openai/gpt-4o"}"#, "{}"),
            r#"{"max_output_tokens":300}"#,
        ),
        deployment(
            "group",
            r#"{"model":"openai/gpt-4o"}"#,
            r#"{"max_output_tokens":200}"#,
        ),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let placement_service = service(&config, &deployments, &info, &counter);
    assert_eq!(placement_service.group_output_ceiling("group"), Some(200));
}

fn an_undeclared_output_limit_removes_the_ceiling() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment(
            "group",
            r#"{"model":"openai/gpt-4o"}"#,
            r#"{"max_output_tokens":100}"#,
        ),
        deployment("group", r#"{"model":"openai/unknown"}"#, "{}"),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let placement_service = service(&config, &deployments, &info, &counter);
    assert_eq!(placement_service.group_output_ceiling("group"), None);
}

fn a_declared_limit_beats_the_router_info() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![with_router(
        deployment("group", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        r#"{"max_input_tokens":9999}"#,
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let placement_service = service(&config, &deployments, &info, &counter);
    assert_eq!(
        placement_service.group_window_facts("group"),
        WindowFacts {
            window: Some(100),
            has_unknown: false,
        }
    );
}

fn an_authenticating_provider_has_no_router_limit() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        with_router(
            deployment("copilot", r#"{"model":"github_copilot/gpt-4o"}"#, "{}"),
            r#"{"max_input_tokens":800}"#,
        ),
        with_router(
            deployment(
                "declared",
                r#"{"model":"github_copilot/gpt-4o"}"#,
                &window(120),
            ),
            r#"{"max_input_tokens":800}"#,
        ),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let placement_service = service(&config, &deployments, &info, &counter);
    assert_eq!(
        placement_service.group_window_facts("copilot"),
        WindowFacts {
            window: None,
            has_unknown: true,
        }
    );
    assert_eq!(
        placement_service.group_window_facts("declared"),
        WindowFacts {
            window: Some(120),
            has_unknown: false,
        }
    );
}

fn a_custom_provider_overrides_the_model_prefix() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        with_router(
            deployment(
                "overridden",
                r#"{"model":"github_copilot/gpt-4o","custom_llm_provider":"openai"}"#,
                "{}",
            ),
            r#"{"max_input_tokens":800}"#,
        ),
        with_router(
            deployment(
                "flagged",
                r#"{"model":"github_copilot/gpt-4o","custom_llm_provider":""}"#,
                "{}",
            ),
            r#"{"max_input_tokens":800}"#,
        ),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let placement_service = service(&config, &deployments, &info, &counter);
    assert_eq!(
        placement_service.group_window_facts("overridden"),
        WindowFacts {
            window: Some(800),
            has_unknown: false,
        }
    );
    assert_eq!(
        placement_service.group_window_facts("flagged"),
        WindowFacts {
            window: None,
            has_unknown: true,
        }
    );
}

fn catalog_limits_resolve_for_a_declared_model() {
    let catalog = model_catalog(
        r#"{"gpt-4o":{"litellm_provider":"openai","mode":"chat","max_input_tokens":1000,"max_output_tokens":200}}"#,
    );
    let info = CatalogModelInfo { catalog: &catalog };
    let spec = deployment("group", r#"{"model":"openai/gpt-4o"}"#, "{}");
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::InputTokens),
        Some(1000)
    );
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::OutputTokens),
        Some(200)
    );
}

fn catalog_azure_resolves_by_base_model() {
    let catalog = model_catalog(
        r#"{"azure/gpt-4o":{"litellm_provider":"azure","mode":"chat","max_input_tokens":128000}}"#,
    );
    let info = CatalogModelInfo { catalog: &catalog };
    let spec = deployment(
        "group",
        r#"{"model":"azure/my-deployment","base_model":"gpt-4o"}"#,
        "{}",
    );
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::InputTokens),
        Some(128000)
    );
}

fn catalog_unmapped_models_have_no_limit() {
    let catalog = model_catalog(
        r#"{"gpt-4o":{"litellm_provider":"openai","mode":"chat","max_input_tokens":1000}}"#,
    );
    let info = CatalogModelInfo { catalog: &catalog };
    let unknown = deployment("group", r#"{"model":"openai/unmapped"}"#, "{}");
    assert_eq!(
        deployment_limit(&info, "group", &unknown, ModelLimitKey::InputTokens),
        None
    );
    let missing = deployment("group", "{}", "{}");
    assert_eq!(
        deployment_limit(&info, "group", &missing, ModelLimitKey::InputTokens),
        None
    );
}

fn message_without_content_adds_no_bytes() {
    let bare = betterlite_openai::chat::ChatMessage {
        role: betterlite_openai::chat::Role::User,
        content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: indexmap::IndexMap::new(),
    };
    let out_of_band = OutOfBand::default();
    assert_eq!(request_byte_upper_bound(Some(&[bare]), &out_of_band), 0);
    assert_eq!(request_byte_upper_bound(None, &out_of_band), 0);
}

fn message_parts_measure_as_their_json() {
    let mut fields = indexmap::IndexMap::new();
    fields.insert("text".to_owned(), serde_json::json!("hi"));
    let part = betterlite_openai::chat::ContentPart {
        kind: "text".to_owned(),
        fields,
    };
    let parts = betterlite_openai::chat::ChatMessage {
        role: betterlite_openai::chat::Role::User,
        content: Some(betterlite_openai::chat::MessageContent::Parts(vec![part])),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: indexmap::IndexMap::new(),
    };
    let out_of_band = OutOfBand::default();
    // [{"type":"text","text":"hi"}] in bytes
    assert_eq!(request_byte_upper_bound(Some(&[parts]), &out_of_band), 29);
}

fn a_match_by_name_returns_the_placement_tier() {
    let placement = Placement {
        tier: Tier::Named("SIMPLE".to_owned()),
        allowed_models: vec!["a".to_owned()],
        holdable_models: BTreeSet::new(),
    };
    let applied = apply_context_placement(
        Tier::BuiltIn(ComplexityTier::Simple),
        vec!["keep".to_owned()],
        Some(&placement),
    );
    assert_eq!(applied.tier, Tier::Named("SIMPLE".to_owned()));
    assert_eq!(applied.signals, vec!["keep".to_owned()]);
    assert_eq!(applied.original_tier, None);
}

fn no_placement_leaves_the_tier_and_signals() {
    let applied = apply_context_placement(
        Tier::BuiltIn(ComplexityTier::Medium),
        vec!["keep".to_owned()],
        None,
    );
    assert_eq!(applied.tier, Tier::BuiltIn(ComplexityTier::Medium));
    assert_eq!(applied.signals, vec!["keep".to_owned()]);
    assert_eq!(applied.original_tier, None);
}

fn the_fit_accepts_an_unknown_group() {
    let fit = ContextFit {
        facts: BTreeMap::new(),
        needed: Some(500),
        buffer: 0.95,
    };
    assert!(fit.accepts("anything"));
    let uncounted = ContextFit {
        facts: BTreeMap::from([(
            "small".to_owned(),
            WindowFacts {
                window: Some(100),
                has_unknown: false,
            },
        )]),
        needed: None,
        buffer: 0.95,
    };
    assert!(uncounted.accepts("small"));
    let known = ContextFit {
        facts: BTreeMap::from([(
            "small".to_owned(),
            WindowFacts {
                window: Some(100),
                has_unknown: false,
            },
        )]),
        needed: Some(96),
        buffer: 0.95,
    };
    assert!(!known.accepts("small"));
    assert!(known.accepts("other"));
}

// a caller can hand the placement a fit it computed under another gate state, and the gate still decides
fn a_disabled_gate_ignores_a_supplied_fit() {
    let config = config(false, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("large", r#"{"model":"openai/gpt-4o"}"#, &window(10_000)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 96 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"]), ("MEDIUM", &["large"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let supplied = ContextFit {
        facts: BTreeMap::from([
            (
                "small".to_owned(),
                WindowFacts {
                    window: Some(100),
                    has_unknown: false,
                },
            ),
            (
                "large".to_owned(),
                WindowFacts {
                    window: Some(10_000),
                    has_unknown: false,
                },
            ),
        ]),
        needed: Some(96),
        buffer: 0.95,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let outcome = super::runtime()
        .block_on(placement_service.context_window_placement(&input, Some(&supplied)));
    assert_eq!(outcome.unwrap(), None);
}

fn catalog_custom_provider_overrides_the_model_prefix() {
    let catalog = model_catalog(
        r#"{"gpt-4o":{"litellm_provider":"openai","mode":"chat","max_input_tokens":1000}}"#,
    );
    let info = CatalogModelInfo { catalog: &catalog };
    let spec = deployment(
        "group",
        r#"{"model":"azure/gpt-4o","custom_llm_provider":"openai"}"#,
        "{}",
    );
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::InputTokens),
        Some(1000)
    );
}

fn an_empty_parts_message_adds_no_bytes() {
    let empty = betterlite_openai::chat::ChatMessage {
        role: betterlite_openai::chat::Role::User,
        content: Some(betterlite_openai::chat::MessageContent::Parts(Vec::new())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: indexmap::IndexMap::new(),
    };
    let out_of_band = OutOfBand::default();
    assert_eq!(request_byte_upper_bound(Some(&[empty]), &out_of_band), 0);
}

// Python's truthiness decides which tools value is carried: text and a mapping count, a zero does not
fn the_tool_truthiness_follows_python_values() {
    let text = Json::Text("tool".to_owned());
    let object = json(r#"{"a":1}"#);
    let zero = json("0");
    let from_text = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&text),
        request_tools: None,
    };
    assert_eq!(out_of_band_text(&from_text), "tool");
    let from_object = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&object),
        request_tools: None,
    };
    assert_eq!(out_of_band_text(&from_object), r#"{"a":1}"#);
    let from_zero = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&zero),
        request_tools: None,
    };
    assert_eq!(out_of_band_text(&from_zero), "");
}

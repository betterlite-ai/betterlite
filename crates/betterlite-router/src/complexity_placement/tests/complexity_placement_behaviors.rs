use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::complexity_placement::RetainedPin;
use crate::complexity_placement::TierChoice;
use crate::complexity_placement::params::params_for_model;
use crate::complexity_placement::params_on_request::{
    TierTarget, mark_raw_model_name, pin_tier_params_onto_request, restore_ceiling_without_pin,
};
use crate::complexity_placement::pin::restamp_adaptive_choice;
use crate::complexity_placement::pools::{model_for_tier, tier_pools};
use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::tier_definitions::model::TierPool;

use super::support::{
    HALF, LAST_SAMPLE, MEDIUM, NoDraw, ScriptedRng, Support, THREE_QUARTERS, World, deployment,
    entry, number, pool, request, runtime, single, text,
};

#[test]
fn a_tier_pool_draws_by_the_injected_sample() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b", "c"]));
    let context = world.context();
    let service = world.service(&context);
    let mut random = ScriptedRng::new(&[HALF]);
    let chosen = model_for_tier(&service, &MEDIUM, &mut random).unwrap();
    assert_eq!(chosen, "b");
    assert_eq!(random.draws, 1);
}

#[test]
fn a_single_tier_model_is_returned_without_a_draw() {
    let mut world = World::new();
    world.set_pool("MEDIUM", single("mid"));
    let context = world.context();
    let service = world.service(&context);
    let mut no_draw = NoDraw;
    let chosen = model_for_tier(&service, &MEDIUM, &mut no_draw).unwrap();
    assert_eq!(chosen, "mid");
}

#[test]
fn a_missing_tier_uses_the_default_model() {
    let mut world = World::new();
    world.default_model = Some("fallback".to_owned());
    let context = world.context();
    let service = world.service(&context);
    let mut no_draw = NoDraw;
    let chosen = model_for_tier(&service, &MEDIUM, &mut no_draw).unwrap();
    assert_eq!(chosen, "fallback");
}

#[test]
fn a_missing_tier_and_default_uses_the_medium_pool() {
    let mut world = World::new();
    world.set_pool("MEDIUM", single("mid"));
    let context = world.context();
    let service = world.service(&context);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let mut no_draw = NoDraw;
    let chosen = model_for_tier(&service, &tier, &mut no_draw).unwrap();
    assert_eq!(chosen, "mid");
}

#[test]
fn a_placed_tier_carries_its_entry_params_and_the_group_ceiling() {
    let mut world = World::new();
    world.set_pool(
        "MEDIUM",
        TierPool::Single(entry("mid", &[("temperature", number(2))])),
    );
    world.add("mid", "openai/gpt-4o", Some(1000));
    let context = world.context();
    let service = world.service(&context);
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: None,
    };
    let mut caller = request(&[]);
    let mut no_draw = NoDraw;
    let placed = runtime()
        .block_on(service.place_tier(&choice, &mut caller, &mut no_draw))
        .unwrap();
    assert_eq!(placed.tier, "MEDIUM");
    assert_eq!(placed.model, "mid");
    assert_eq!(placed.candidates, vec!["mid".to_owned()]);
    assert_eq!(placed.params.get("temperature"), Some(&number(2)));
    assert_eq!(placed.params.get("max_tokens"), Some(&number(1000)));
    assert!(caller.is_empty());
}

#[test]
fn a_drawn_pool_entry_brings_its_own_params() {
    let mut world = World::new();
    world.set_pool(
        "MEDIUM",
        TierPool::Pool(vec![
            entry("a", &[("temperature", number(1))]),
            entry("b", &[("temperature", number(2))]),
        ]),
    );
    let context = world.context();
    let service = world.service(&context);
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: None,
    };
    let mut caller = request(&[]);
    let mut random = ScriptedRng::new(&[LAST_SAMPLE]);
    let placed = runtime()
        .block_on(service.place_tier(&choice, &mut caller, &mut random))
        .unwrap();
    assert_eq!(placed.model, "b");
    assert_eq!(placed.params.get("temperature"), Some(&number(2)));
    assert_eq!(
        params_for_model(&service, Some(&MEDIUM), "a").get("temperature"),
        Some(&number(1))
    );
}

#[test]
fn an_explicit_tier_ceiling_keeps_its_own_max_tokens() {
    let mut world = World::new();
    world.set_pool(
        "MEDIUM",
        TierPool::Single(entry("mid", &[("max_tokens", number(7))])),
    );
    world.add("mid", "openai/gpt-4o", Some(1000));
    let context = world.context();
    let service = world.service(&context);
    let params = params_for_model(&service, Some(&MEDIUM), "mid");
    assert_eq!(params.get("max_tokens"), Some(&number(7)));
}

#[test]
fn the_tier_model_ceiling_can_be_switched_off() {
    let mut world = World::new();
    world.config.max_tokens_from_tier_model = false;
    world.set_pool("MEDIUM", single("mid"));
    world.add("mid", "openai/gpt-4o", Some(1000));
    let context = world.context();
    let service = world.service(&context);
    let params = params_for_model(&service, Some(&MEDIUM), "mid");
    assert!(params.is_empty());
}

#[test]
fn a_model_outside_every_tier_still_gets_the_group_ceiling() {
    let mut world = World::new();
    world.add("drawn", "openai/gpt-4o", Some(500));
    let context = world.context();
    let service = world.service(&context);
    let params = params_for_model(&service, None, "drawn");
    assert_eq!(params.get("max_tokens"), Some(&number(500)));
}

#[test]
fn a_caller_pool_override_draws_from_the_override() {
    let mut world = World::new();
    world.add("x", "openai/gpt-4o", Some(500));
    let context = world.context();
    let service = world.service(&context);
    let allowed = vec!["x".to_owned()];
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: Some(&allowed),
        retained_pin: None,
    };
    let mut caller = request(&[]);
    let mut random = ScriptedRng::new(&[LAST_SAMPLE]);
    let placed = runtime()
        .block_on(service.place_tier(&choice, &mut caller, &mut random))
        .unwrap();
    assert_eq!(placed.model, "x");
    assert_eq!(placed.candidates, allowed);
    assert_eq!(placed.params.get("max_tokens"), Some(&number(500)));
}

#[test]
fn a_session_keeps_its_first_model_and_restamps_the_marker() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: None,
    };
    let mut first = request(&[(
        "metadata",
        Json::Object(request(&[("session_id", text("s1"))])),
    )]);
    let mut first_random = ScriptedRng::new(&[0]);
    let first_placed = runtime()
        .block_on(service.place_tier(&choice, &mut first, &mut first_random))
        .unwrap();
    assert_eq!(first_placed.model, "a");
    let mut second = request(&[(
        "metadata",
        Json::Object(request(&[
            ("session_id", text("s1")),
            ("adaptive_router_chosen_model", text("b")),
        ])),
    )]);
    let mut second_random = ScriptedRng::new(&[LAST_SAMPLE]);
    let second_placed = runtime()
        .block_on(service.place_tier(&choice, &mut second, &mut second_random))
        .unwrap();
    assert_eq!(second_placed.model, "a");
    assert_eq!(
        second.get("metadata"),
        Some(&Json::Object(request(&[
            ("session_id", text("s1")),
            ("adaptive_router_chosen_model", text("a")),
        ])))
    );
}

#[test]
fn a_restamp_moves_the_nested_decision_model_too() {
    let mut caller = request(&[(
        "metadata",
        Json::Object(request(&[
            (
                "adaptive_router_decision",
                Json::Object(request(&[("chosen_model", text("b"))])),
            ),
            ("adaptive_router_chosen_model", text("elsewhere")),
        ])),
    )]);
    restamp_adaptive_choice(&mut caller, "b", "a");
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[
            (
                "adaptive_router_decision",
                Json::Object(request(&[("chosen_model", text("a"))])),
            ),
            ("adaptive_router_chosen_model", text("elsewhere")),
        ])))
    );
    let mut untouched = request(&[(
        "metadata",
        Json::Object(request(&[("adaptive_router_decision", text("nope"))])),
    )]);
    restamp_adaptive_choice(&mut untouched, "b", "a");
    assert_eq!(
        untouched.get("metadata"),
        Some(&Json::Object(request(&[(
            "adaptive_router_decision",
            text("nope"),
        )])))
    );
}

#[test]
fn a_retained_pin_wins_over_the_draw() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let retained = RetainedPin {
        model: "b".to_owned(),
        tier: Some("MEDIUM".to_owned()),
    };
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: Some(&retained),
    };
    let mut caller = request(&[(
        "metadata",
        Json::Object(request(&[("adaptive_router_chosen_model", text("a"))])),
    )]);
    let mut random = ScriptedRng::new(&[0]);
    let placed = runtime()
        .block_on(service.place_tier(&choice, &mut caller, &mut random))
        .unwrap();
    assert_eq!(placed.model, "b");
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[(
            "adaptive_router_chosen_model",
            text("b"),
        )])))
    );
}

#[test]
fn a_retained_pin_of_another_tier_is_ignored() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let retained = RetainedPin {
        model: "b".to_owned(),
        tier: Some("SIMPLE".to_owned()),
    };
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: Some(&retained),
    };
    let mut caller = request(&[]);
    let mut random = ScriptedRng::new(&[0]);
    let placed = runtime()
        .block_on(service.place_tier(&choice, &mut caller, &mut random))
        .unwrap();
    assert_eq!(placed.model, "a");
}

#[test]
fn a_retained_pin_outside_the_candidates_is_ignored() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let retained = RetainedPin {
        model: "z".to_owned(),
        tier: Some("MEDIUM".to_owned()),
    };
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: Some(&retained),
    };
    let mut caller = request(&[]);
    let mut random = ScriptedRng::new(&[0]);
    let placed = runtime()
        .block_on(service.place_tier(&choice, &mut caller, &mut random))
        .unwrap();
    assert_eq!(placed.model, "a");
}

#[test]
fn a_pin_does_not_apply_without_the_deployment_pin_flag() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    let context = world.context();
    let service = world.service(&context);
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: None,
    };
    let mut first = request(&[(
        "metadata",
        Json::Object(request(&[("session_id", text("s1"))])),
    )]);
    let mut first_random = ScriptedRng::new(&[0]);
    let first_placed = runtime()
        .block_on(service.place_tier(&choice, &mut first, &mut first_random))
        .unwrap();
    assert_eq!(first_placed.model, "a");
    let mut second = request(&[(
        "metadata",
        Json::Object(request(&[("session_id", text("s1"))])),
    )]);
    let mut second_random = ScriptedRng::new(&[LAST_SAMPLE]);
    let second_placed = runtime()
        .block_on(service.place_tier(&choice, &mut second, &mut second_random))
        .unwrap();
    assert_eq!(second_placed.model, "b");
}

#[test]
fn two_callers_of_one_session_do_not_share_a_tier_pin() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: None,
    };
    let mut first = request(&[(
        "metadata",
        Json::Object(request(&[
            ("session_id", text("s1")),
            ("user_api_key_hash", text("caller-a")),
        ])),
    )]);
    let mut first_random = ScriptedRng::new(&[0]);
    let first_placed = runtime()
        .block_on(service.place_tier(&choice, &mut first, &mut first_random))
        .unwrap();
    assert_eq!(first_placed.model, "a");
    let mut second = request(&[(
        "metadata",
        Json::Object(request(&[
            ("session_id", text("s1")),
            ("user_api_key_hash", text("caller-b")),
        ])),
    )]);
    let mut second_random = ScriptedRng::new(&[LAST_SAMPLE]);
    let second_placed = runtime()
        .block_on(service.place_tier(&choice, &mut second, &mut second_random))
        .unwrap();
    assert_eq!(second_placed.model, "b");
}

#[test]
fn the_tier_params_apply_over_the_callers_own() {
    let mut world = World::new();
    world.add("mid", "openai/gpt-4o", Some(1000));
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(&[("temperature", number(2))]);
    let mut caller = request(&[("temperature", number(1)), ("top_p", number(1))]);
    let pinned =
        pin_tier_params_onto_request(&target, "mid", &tier_params, &mut caller, false).unwrap();
    assert!(!pinned);
    assert_eq!(caller.get("temperature"), Some(&number(2)));
    assert_eq!(caller.get("top_p"), Some(&number(1)));
}

#[test]
fn a_tier_ceiling_collapses_onto_max_tokens_and_stamps_the_caller_ceiling() {
    let mut world = World::new();
    world.add("mid", "openai/gpt-4o", Some(1000));
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(&[("max_completion_tokens", number(100))]);
    let mut caller = request(&[
        ("max_tokens", number(99)),
        ("max_output_tokens", number(88)),
    ]);
    let pinned =
        pin_tier_params_onto_request(&target, "mid", &tier_params, &mut caller, false).unwrap();
    assert!(pinned);
    assert_eq!(caller.get("max_tokens"), Some(&number(100)));
    assert_eq!(caller.get("max_completion_tokens"), None);
    assert_eq!(caller.get("max_output_tokens"), None);
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[(
            "_client_output_ceiling",
            Json::Object(request(&[
                ("max_tokens", number(99)),
                ("max_output_tokens", number(88)),
            ])),
        )])))
    );
}

#[test]
fn a_responses_call_pins_max_output_tokens() {
    let mut world = World::new();
    world.add("mid", "openai/gpt-4o", Some(1000));
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(&[("max_completion_tokens", number(100))]);
    let mut caller = request(&[]);
    let pinned =
        pin_tier_params_onto_request(&target, "mid", &tier_params, &mut caller, true).unwrap();
    assert!(pinned);
    assert_eq!(caller.get("max_output_tokens"), Some(&number(100)));
    assert_eq!(caller.get("max_tokens"), None);
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[(
            "_client_output_ceiling",
            Json::Object(request(&[])),
        )])))
    );
}

#[test]
fn a_non_pinning_pass_restores_the_callers_ceiling() {
    let mut world = World::new();
    world.add("mid", "openai/gpt-4o", Some(1000));
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(&[("max_tokens", number(100))]);
    let mut caller = request(&[("max_tokens", number(99))]);
    let first_pinned =
        pin_tier_params_onto_request(&target, "mid", &tier_params, &mut caller, false).unwrap();
    assert!(first_pinned);
    let second_pinned =
        pin_tier_params_onto_request(&target, "mid", &request(&[]), &mut caller, false).unwrap();
    assert!(!second_pinned);
    restore_ceiling_without_pin(&mut caller);
    assert_eq!(caller.get("max_tokens"), Some(&number(99)));
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[(
            "_client_output_ceiling",
            Json::Object(request(&[("max_tokens", number(99))])),
        )])))
    );
}

#[test]
fn a_tier_effort_drops_the_clients_other_carriers() {
    let mut world = World::new();
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(&[("reasoning_effort", text("high"))]);
    let mut caller = request(&[
        ("reasoning_effort", text("low")),
        (
            "thinking",
            Json::Object(request(&[("type", text("enabled"))])),
        ),
        (
            "output_config",
            Json::Object(request(&[
                ("effort", text("low")),
                ("format", text("json")),
            ])),
        ),
        (
            "reasoning",
            Json::Object(request(&[("effort", text("low"))])),
        ),
    ]);
    let pinned =
        pin_tier_params_onto_request(&target, "mid", &tier_params, &mut caller, false).unwrap();
    assert!(!pinned);
    assert_eq!(caller.get("reasoning_effort"), Some(&text("high")));
    assert_eq!(caller.get("thinking"), None);
    assert_eq!(caller.get("reasoning"), None);
    assert_eq!(
        caller.get("output_config"),
        Some(&Json::Object(request(&[("format", text("json"))])))
    );
}

#[test]
fn a_tier_param_no_deployment_takes_is_dropped() {
    let mut world = World::new();
    world.add("mid", "openai/gpt-4o", Some(1000));
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(&[("temperature", number(2)), ("top_p", number(1))]);
    let mut caller = request(&[]);
    let pinned =
        pin_tier_params_onto_request(&target, "mid", &tier_params, &mut caller, false).unwrap();
    assert!(!pinned);
    assert_eq!(caller.get("temperature"), Some(&number(2)));
    assert_eq!(caller.get("top_p"), None);
}

#[test]
fn a_deployment_allowlist_keeps_the_tier_param() {
    let mut world = World::new();
    world
        .catalog
        .push(deployment("mid", "openai/gpt-4o", Some(1000), &["top_p"]));
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(&[("top_p", number(1))]);
    let mut caller = request(&[]);
    pin_tier_params_onto_request(&target, "mid", &tier_params, &mut caller, false).unwrap();
    assert_eq!(caller.get("top_p"), Some(&number(1)));
}

#[test]
fn a_model_without_deployments_keeps_every_tier_param() {
    let world = World::new();
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(&[("top_p", number(1))]);
    let mut caller = request(&[]);
    pin_tier_params_onto_request(&target, "ghost", &tier_params, &mut caller, false).unwrap();
    assert_eq!(caller.get("top_p"), Some(&number(1)));
}

#[test]
fn the_raw_model_name_flag_is_stamped_when_configured() {
    let mut world = World::new();
    world.config.return_raw_model_name = true;
    let context = world.context();
    let service = world.service(&context);
    let mut caller = request(&[("metadata", Json::Object(request(&[("kept", text("yes"))])))]);
    mark_raw_model_name(service.config, &mut caller);
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[
            ("kept", text("yes")),
            ("_complexity_router_return_raw_model_name", Json::Bool(true)),
        ])))
    );
}

#[test]
fn the_raw_model_name_flag_uses_the_litellm_metadata_bucket() {
    let mut world = World::new();
    world.config.return_raw_model_name = true;
    let context = world.context();
    let service = world.service(&context);
    let mut caller = request(&[("litellm_metadata", Json::Object(request(&[])))]);
    mark_raw_model_name(service.config, &mut caller);
    assert_eq!(
        caller.get("litellm_metadata"),
        Some(&Json::Object(request(&[(
            "_complexity_router_return_raw_model_name",
            Json::Bool(true),
        )])))
    );
    assert_eq!(caller.get("metadata"), None);
}

#[test]
fn the_raw_model_name_flag_is_absent_when_switched_off() {
    let world = World::new();
    let context = world.context();
    let service = world.service(&context);
    let mut caller = request(&[]);
    mark_raw_model_name(service.config, &mut caller);
    assert!(caller.is_empty());
}

#[test]
fn a_stale_false_raw_model_name_flag_is_overwritten() {
    let mut world = World::new();
    world.config.return_raw_model_name = true;
    let context = world.context();
    let service = world.service(&context);
    let mut caller = request(&[(
        "metadata",
        Json::Object(request(&[(
            "_complexity_router_return_raw_model_name",
            Json::Bool(false),
        )])),
    )]);
    mark_raw_model_name(service.config, &mut caller);
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[(
            "_complexity_router_return_raw_model_name",
            Json::Bool(true),
        )])))
    );
}

#[test]
fn a_non_object_metadata_bucket_is_left_alone() {
    let mut world = World::new();
    world.config.return_raw_model_name = true;
    let context = world.context();
    let service = world.service(&context);
    let mut caller = request(&[("metadata", text("not a bucket"))]);
    mark_raw_model_name(service.config, &mut caller);
    assert_eq!(caller.get("metadata"), Some(&text("not a bucket")));
}

#[test]
fn the_tier_pools_view_names_every_configured_tier() {
    let mut world = World::new();
    world.set_pool("SIMPLE", single("mini"));
    world.set_pool("MEDIUM", pool(&["mid-a", "mid-b"]));
    let context = world.context();
    let service = world.service(&context);
    let pools = tier_pools(service.tiers);
    assert_eq!(pools.get("SIMPLE"), Some(&vec!["mini".to_owned()]));
    assert_eq!(
        pools.get("MEDIUM"),
        Some(&vec!["mid-a".to_owned(), "mid-b".to_owned()])
    );
    assert_eq!(pools.len(), 2);
}

#[test]
fn a_named_tier_reads_its_own_pool() {
    let mut world = World::new();
    world.set_pool("frontier", pool(&["big-a", "big-b"]));
    let context = world.context();
    let service = world.service(&context);
    let tier = Tier::Named("frontier".to_owned());
    let mut random = ScriptedRng::new(&[THREE_QUARTERS]);
    let chosen = model_for_tier(&service, &tier, &mut random).unwrap();
    assert_eq!(chosen, "big-b");
}

#[rstest]
#[case::own_pool_single(ComplexityTier::Medium, Some("mid"), None, None, "mid")]
#[case::default_model(ComplexityTier::Medium, None, None, Some("fallback"), "fallback")]
#[case::medium_pool(ComplexityTier::Simple, None, Some("mid"), None, "mid")]
fn a_tier_resolves_in_litellm_order(
    #[case] tier: ComplexityTier,
    #[case] own_pool: Option<&str>,
    #[case] medium_pool: Option<&str>,
    #[case] default_model: Option<&str>,
    #[case] expected: &str,
) {
    let mut world = World::new();
    if let Some(model) = own_pool {
        world.set_pool(tier.name(), single(model));
    }
    if let Some(model) = medium_pool {
        world.set_pool("MEDIUM", single(model));
    }
    if let Some(model) = default_model {
        world.default_model = Some(model.to_owned());
    }
    let context = world.context();
    let service = world.service(&context);
    let mut no_draw = NoDraw;
    let chosen = model_for_tier(&service, &Tier::BuiltIn(tier), &mut no_draw).unwrap();
    assert_eq!(chosen, expected);
}

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;
use sha2::{Digest as _, Sha256};

use crate::affinity_claims::pin::{AffinityPin, StoredPin};
use crate::complexity_placement::Selection;
use crate::complexity_placement::error::TierPlacementError;
use crate::complexity_placement::params_on_request::{
    TierTarget, pin_tier_params_onto_request, restore_ceiling_without_pin,
};
use crate::complexity_placement::pin::{claimed_model, pin_model_for_tier, tier_pin_cache_key};
use crate::complexity_placement::pools::{
    draw_index, entry_model, model_for_tier, pick_from_names, pick_from_pool,
};
use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::model_catalog_lookup::alias::AliasTarget;
use crate::tier_definitions::model::TierPool;

use super::super::TierChoice;
use super::support::{
    HALF, LAST_SAMPLE, MEDIUM, NoDraw, QUARTER, ScriptedRng, Support, THREE_QUARTERS, World, ZERO,
    entry, number, pool, request, runtime, single, text,
};

// floor(2^53 / 3) and one past it, floor(2 * 2^53 / 3) and one below, 2^52 - 1: where rounding reaches the next index
const THIRD: u64 = 3_002_399_751_580_330;
const THIRD_PLUS_ONE: u64 = 3_002_399_751_580_331;
const TWO_THIRDS: u64 = 6_004_799_503_160_661;
const TWO_THIRDS_MINUS_ONE: u64 = 6_004_799_503_160_660;
const HALF_MINUS_ONE: u64 = 0x000F_FFFF_FFFF_FFFF;

fn pin_of(model: &str) -> StoredPin {
    let mut fields = IndexMap::new();
    fields.insert("model".to_owned(), model.to_owned());
    StoredPin::Pin(AffinityPin(fields))
}

#[rstest]
#[case::zero(ZERO, 0)]
#[case::quarter(QUARTER, 0)]
#[case::half(HALF, 1)]
#[case::three_quarters(THREE_QUARTERS, 2)]
#[case::last(LAST_SAMPLE, 2)]
fn a_three_model_draw_lands_like_random_choice(#[case] sample: u64, #[case] expected: usize) {
    let mut random = ScriptedRng::new(&[sample]);
    assert_eq!(draw_index(3, &mut random).unwrap(), expected);
}

#[rstest]
#[case::just_below(HALF_MINUS_ONE, "a")]
#[case::at_half(HALF, "b")]
fn a_two_model_pool_switches_at_half(#[case] sample: u64, #[case] expected: &str) {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    let context = world.context();
    let service = world.service(&context);
    let mut random = ScriptedRng::new(&[sample]);
    let chosen = model_for_tier(&service, &MEDIUM, &mut random).unwrap();
    assert_eq!(chosen, expected);
}

#[rstest]
#[case::at_third(THIRD, 0)]
#[case::above_third(THIRD_PLUS_ONE, 1)]
#[case::below_two_thirds(TWO_THIRDS_MINUS_ONE, 1)]
#[case::at_two_thirds(TWO_THIRDS, 2)]
fn a_three_model_draw_switches_at_each_third(#[case] sample: u64, #[case] expected: usize) {
    let mut random = ScriptedRng::new(&[sample]);
    assert_eq!(draw_index(3, &mut random).unwrap(), expected);
}

#[test]
fn a_one_entry_list_still_draws() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["only"]));
    let context = world.context();
    let service = world.service(&context);
    let mut random = ScriptedRng::new(&[LAST_SAMPLE]);
    let chosen = model_for_tier(&service, &MEDIUM, &mut random).unwrap();
    assert_eq!(chosen, "only");
    assert_eq!(random.draws, 1);
}

#[test]
fn an_empty_pool_names_the_requested_tier() {
    let mut no_draw = NoDraw;
    let error = pick_from_pool(&TierPool::Pool(Vec::new()), "SIMPLE", &mut no_draw).unwrap_err();
    assert_eq!(error.to_string(), "Empty model pool for tier SIMPLE");
    assert!(matches!(error, TierPlacementError::EmptyPool { .. }));
}

#[test]
fn a_missing_tier_without_a_default_names_the_tier() {
    let world = World::new();
    let context = world.context();
    let service = world.service(&context);
    let mut no_draw = NoDraw;
    let error = model_for_tier(&service, &MEDIUM, &mut no_draw).unwrap_err();
    assert_eq!(
        error.to_string(),
        "No model configured for tier MEDIUM and no default_model set"
    );
    assert!(matches!(error, TierPlacementError::NoModelForTier { .. }));
}

#[test]
fn an_empty_medium_pool_is_reported_under_medium() {
    let mut world = World::new();
    world.set_pool("MEDIUM", TierPool::Pool(Vec::new()));
    let context = world.context();
    let service = world.service(&context);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let mut no_draw = NoDraw;
    let error = model_for_tier(&service, &tier, &mut no_draw).unwrap_err();
    assert_eq!(error.to_string(), "Empty model pool for tier MEDIUM");
}

#[test]
fn an_empty_default_model_is_used_as_given() {
    let mut world = World::new();
    world.default_model = Some(String::new());
    let context = world.context();
    let service = world.service(&context);
    let mut no_draw = NoDraw;
    let chosen = model_for_tier(&service, &MEDIUM, &mut no_draw).unwrap();
    assert_eq!(chosen, "");
}

#[test]
fn a_lowercase_pool_key_is_not_the_built_in_tier() {
    let mut world = World::new();
    world.set_pool("simple", single("lower"));
    let context = world.context();
    let service = world.service(&context);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let mut no_draw = NoDraw;
    let error = model_for_tier(&service, &tier, &mut no_draw).unwrap_err();
    assert_eq!(
        error.to_string(),
        "No model configured for tier SIMPLE and no default_model set"
    );
}

#[test]
fn a_single_entry_with_an_empty_name_is_returned_as_given() {
    let mut world = World::new();
    world.set_pool("MEDIUM", TierPool::Single(entry("", &[])));
    let context = world.context();
    let service = world.service(&context);
    let mut no_draw = NoDraw;
    let chosen = model_for_tier(&service, &MEDIUM, &mut no_draw).unwrap();
    assert_eq!(chosen, "");
}

#[test]
fn a_draw_past_the_last_entry_is_refused() {
    let entries = vec![entry("a", &[])];
    assert!(matches!(
        entry_model(&entries, 1),
        Err(TierPlacementError::Draw)
    ));
    assert!(matches!(entry_model(&[], 0), Err(TierPlacementError::Draw)));
}

#[test]
fn an_empty_caller_pool_is_refused_before_any_draw() {
    let mut no_draw = NoDraw;
    let error = pick_from_names(&[], "MEDIUM", &mut no_draw).unwrap_err();
    assert_eq!(error.to_string(), "Empty model pool for tier MEDIUM");
}

#[test]
fn a_model_outside_the_candidates_writes_no_pin() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let candidates = vec!["a".to_owned()];
    let selection = Selection {
        model: "zzz",
        candidates: &candidates,
    };
    let mut caller = request(&[(
        "metadata",
        Json::Object(request(&[("session_id", text("s1"))])),
    )]);
    let returned = runtime()
        .block_on(pin_model_for_tier(
            &service,
            &MEDIUM,
            &selection,
            &mut caller,
            None,
        ))
        .unwrap();
    assert_eq!(returned, "zzz");
    let key = tier_pin_cache_key(&service, &MEDIUM, "s1", None, None).unwrap();
    assert_eq!(world.claims.local.read(&key).unwrap(), None);
}

#[test]
fn a_session_without_a_caller_key_still_pins() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let candidates = vec!["a".to_owned(), "b".to_owned()];
    let selection = Selection {
        model: "a",
        candidates: &candidates,
    };
    let mut caller = request(&[(
        "metadata",
        Json::Object(request(&[("session_id", text("s1"))])),
    )]);
    let returned = runtime()
        .block_on(pin_model_for_tier(
            &service,
            &MEDIUM,
            &selection,
            &mut caller,
            None,
        ))
        .unwrap();
    assert_eq!(returned, "a");
    let key = tier_pin_cache_key(&service, &MEDIUM, "s1", None, None).unwrap();
    assert_eq!(world.claims.local.read(&key).unwrap(), Some(pin_of("a")));
}

#[test]
fn an_ineligible_seeded_pin_is_replaced() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let key = tier_pin_cache_key(&service, &MEDIUM, "s1", None, None).unwrap();
    world.claims.local.set(&key, &pin_of("zzz"), 60).unwrap();
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: None,
    };
    let mut caller = request(&[(
        "metadata",
        Json::Object(request(&[("session_id", text("s1"))])),
    )]);
    let mut random = ScriptedRng::new(&[0]);
    let placed = runtime()
        .block_on(service.place_tier(&choice, &mut caller, &mut random))
        .unwrap();
    assert_eq!(placed.model, "a");
    assert_eq!(world.claims.local.read(&key).unwrap(), Some(pin_of("a")));
}

#[test]
fn a_seeded_bare_string_pin_is_replaced() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["a", "b"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let key = tier_pin_cache_key(&service, &MEDIUM, "s1", None, None).unwrap();
    world
        .claims
        .local
        .set(&key, &StoredPin::Text("a".to_owned()), 60)
        .unwrap();
    let choice = TierChoice {
        tier: &MEDIUM,
        allowed_models: None,
        retained_pin: None,
    };
    let mut caller = request(&[(
        "metadata",
        Json::Object(request(&[("session_id", text("s1"))])),
    )]);
    let mut random = ScriptedRng::new(&[0]);
    let placed = runtime()
        .block_on(service.place_tier(&choice, &mut caller, &mut random))
        .unwrap();
    assert_eq!(placed.model, "a");
    assert_eq!(world.claims.local.read(&key).unwrap(), Some(pin_of("a")));
}

#[test]
fn a_stamped_ceiling_keeps_the_first_stamp() {
    let mut world = World::new();
    world.add("mid", "openai/gpt-4o", Some(1000));
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let mut caller = request(&[("max_tokens", number(99))]);
    pin_tier_params_onto_request(
        &target,
        "mid",
        &request(&[("max_tokens", number(100))]),
        &mut caller,
        false,
    )
    .unwrap();
    pin_tier_params_onto_request(
        &target,
        "mid",
        &request(&[("max_tokens", number(200))]),
        &mut caller,
        false,
    )
    .unwrap();
    assert_eq!(caller.get("max_tokens"), Some(&number(200)));
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[(
            "_client_output_ceiling",
            Json::Object(request(&[("max_tokens", number(99))])),
        )])))
    );
}

#[test]
fn a_non_object_metadata_bucket_is_replaced_for_a_stamp() {
    let world = World::new();
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let mut caller = request(&[
        ("metadata", text("not a bucket")),
        ("max_output_tokens", number(88)),
    ]);
    pin_tier_params_onto_request(
        &target,
        "mid",
        &request(&[("max_tokens", number(100))]),
        &mut caller,
        false,
    )
    .unwrap();
    assert_eq!(
        caller.get("metadata"),
        Some(&Json::Object(request(&[(
            "_client_output_ceiling",
            Json::Object(request(&[("max_output_tokens", number(88))])),
        )])))
    );
    assert_eq!(caller.get("max_tokens"), Some(&number(100)));
}

#[test]
fn a_non_object_effort_carrier_is_left_alone() {
    let world = World::new();
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let mut caller = request(&[
        ("reasoning", text("low")),
        ("output_config", Json::Object(request(&[]))),
    ]);
    pin_tier_params_onto_request(
        &target,
        "mid",
        &request(&[("reasoning_effort", text("high"))]),
        &mut caller,
        false,
    )
    .unwrap();
    assert_eq!(caller.get("reasoning"), Some(&text("low")));
    assert_eq!(caller.get("output_config"), None);
    assert_eq!(caller.get("reasoning_effort"), Some(&text("high")));
}

#[rstest]
#[case::chat_all_three(
    &[
        ("max_tokens", 1_i64),
        ("max_completion_tokens", 2_i64),
        ("max_output_tokens", 3_i64),
    ],
    false,
    1_i64
)]
#[case::chat_completion_and_output(
    &[("max_completion_tokens", 2_i64), ("max_output_tokens", 3_i64)],
    false,
    2_i64
)]
#[case::chat_output_only(&[("max_output_tokens", 3_i64)], false, 3_i64)]
#[case::responses_output_and_tokens(
    &[("max_output_tokens", 3_i64), ("max_tokens", 1_i64)],
    true,
    3_i64
)]
#[case::responses_tokens_and_completion(
    &[("max_tokens", 1_i64), ("max_completion_tokens", 2_i64)],
    true,
    1_i64
)]
fn a_ceiling_collapses_onto_its_surface_name(
    #[case] params: &[(&str, i64)],
    #[case] responses: bool,
    #[case] expected: i64,
) {
    let world = World::new();
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let tier_params = request(
        &params
            .iter()
            .map(|(key, value)| (*key, number(*value)))
            .collect::<Vec<_>>(),
    );
    let surface = if responses {
        "max_output_tokens"
    } else {
        "max_tokens"
    };
    let mut caller = request(&[]);
    let pinned =
        pin_tier_params_onto_request(&target, "ghost", &tier_params, &mut caller, responses)
            .unwrap();
    assert!(pinned);
    for key in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
        if key == surface {
            assert_eq!(caller.get(key), Some(&number(expected)));
        } else {
            assert_eq!(caller.get(key), None);
        }
    }
}

#[test]
fn an_empty_tier_params_map_changes_nothing() {
    let world = World::new();
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let mut caller = request(&[("max_tokens", number(9))]);
    let pinned =
        pin_tier_params_onto_request(&target, "mid", &request(&[]), &mut caller, false).unwrap();
    assert!(!pinned);
    assert_eq!(caller.get("max_tokens"), Some(&number(9)));
    assert_eq!(caller.get("metadata"), None);
}

#[test]
fn a_restore_takes_the_three_carriers_as_integers() {
    let stamp = Json::Object(request(&[
        (
            "max_tokens",
            Json::Number(Number::from_f64(99.9_f64).expect("finite")),
        ),
        ("max_completion_tokens", text("7")),
        ("max_output_tokens", number(-5)),
    ]));
    let mut caller = request(&[
        ("max_tokens", number(1)),
        ("max_output_tokens", number(2)),
        (
            "metadata",
            Json::Object(request(&[("_client_output_ceiling", stamp)])),
        ),
    ]);
    restore_ceiling_without_pin(&mut caller);
    assert_eq!(caller.get("max_tokens"), Some(&number(99)));
    assert_eq!(caller.get("max_completion_tokens"), Some(&number(7)));
    assert_eq!(caller.get("max_output_tokens"), None);
}

#[test]
fn a_restore_reads_metadata_before_litellm_metadata() {
    let mut caller = request(&[
        (
            "metadata",
            Json::Object(request(&[(
                "_client_output_ceiling",
                Json::Object(request(&[("max_tokens", number(5))])),
            )])),
        ),
        (
            "litellm_metadata",
            Json::Object(request(&[(
                "_client_output_ceiling",
                Json::Object(request(&[("max_tokens", number(6))])),
            )])),
        ),
    ]);
    restore_ceiling_without_pin(&mut caller);
    assert_eq!(caller.get("max_tokens"), Some(&number(5)));
}

#[test]
fn a_restore_ignores_a_non_object_stamp() {
    let mut caller = request(&[
        (
            "metadata",
            Json::Object(request(&[("_client_output_ceiling", text("nope"))])),
        ),
        (
            "litellm_metadata",
            Json::Object(request(&[(
                "_client_output_ceiling",
                Json::Object(request(&[("max_tokens", number(6))])),
            )])),
        ),
        ("max_tokens", number(1)),
    ]);
    restore_ceiling_without_pin(&mut caller);
    assert_eq!(caller.get("max_tokens"), Some(&number(1)));
}

#[test]
fn a_restore_without_a_stamp_leaves_the_carriers() {
    let mut caller = request(&[
        ("metadata", Json::Object(request(&[("kept", text("yes"))]))),
        ("max_tokens", number(1)),
    ]);
    restore_ceiling_without_pin(&mut caller);
    assert_eq!(caller.get("max_tokens"), Some(&number(1)));
}

#[test]
fn the_tier_pin_cache_key_is_the_litellm_digest() {
    let world = World::new();
    let context = world.context();
    let service = world.service(&context);
    let key = tier_pin_cache_key(&service, &MEDIUM, "s1", None, None).unwrap();
    let identity = "[\"router\",\"affinity\",null,\"s1\",\"MEDIUM\"]";
    let expected = format!(
        "complexity_router_tier_model_affinity:v1:{}",
        hex::encode(Sha256::digest(identity.as_bytes()))
    );
    assert_eq!(key.as_ref(), expected.as_str());
    let retained_key = tier_pin_cache_key(&service, &MEDIUM, "s1", None, Some("x")).unwrap();
    let retained_identity =
        "[\"router\",\"affinity\",null,\"s1\",\"MEDIUM\",[\"replay_fallback\",\"x\"]]";
    let retained_expected = format!(
        "complexity_router_tier_model_affinity:v1:{}",
        hex::encode(Sha256::digest(retained_identity.as_bytes()))
    );
    assert_eq!(retained_key.as_ref(), retained_expected.as_str());
}

#[test]
fn two_tier_pin_identities_never_collide() {
    let world = World::new();
    let context = world.context();
    let service = world.service(&context);
    let base = tier_pin_cache_key(&service, &MEDIUM, "s1", None, None).unwrap();
    let caller = tier_pin_cache_key(&service, &MEDIUM, "s1", Some("c"), None).unwrap();
    let session = tier_pin_cache_key(&service, &MEDIUM, "s2", None, None).unwrap();
    let retained = tier_pin_cache_key(&service, &MEDIUM, "s1", None, Some("x")).unwrap();
    let tier = tier_pin_cache_key(
        &service,
        &Tier::BuiltIn(ComplexityTier::Simple),
        "s1",
        None,
        None,
    )
    .unwrap();
    assert_ne!(base, caller);
    assert_ne!(base, session);
    assert_ne!(base, retained);
    assert_ne!(base, tier);
}

#[test]
fn a_lookup_failure_keeps_its_own_error() {
    let mut world = World::new();
    world.aliases.insert(
        "mid".to_owned(),
        AliasTarget::Item {
            model: Some(text("x")),
            hidden: None,
        },
    );
    let lookup = world.lookup();
    let support = Support;
    let target = TierTarget {
        lookup: &lookup,
        support: &support,
    };
    let mut caller = request(&[]);
    let error = pin_tier_params_onto_request(
        &target,
        "mid",
        &request(&[("temperature", number(1))]),
        &mut caller,
        false,
    )
    .unwrap_err();
    assert!(matches!(error, TierPlacementError::Lookup(_)));
}

#[test]
fn claimed_model_reads_only_a_text_model_field() {
    assert_eq!(claimed_model(&pin_of("a")), Some("a"));
    let mut wrong = IndexMap::new();
    wrong.insert("other".to_owned(), "a".to_owned());
    assert_eq!(claimed_model(&StoredPin::Pin(AffinityPin(wrong))), None);
    assert_eq!(claimed_model(&StoredPin::Text("a".to_owned())), None);
    assert_eq!(
        claimed_model(&StoredPin::Other(Json::Object(request(&[(
            "model",
            text("a")
        )])))),
        Some("a")
    );
    assert_eq!(
        claimed_model(&StoredPin::Other(Json::Object(request(&[(
            "model",
            number(5)
        )])))),
        None
    );
    assert_eq!(
        claimed_model(&StoredPin::Other(Json::Object(request(&[])))),
        None
    );
    assert_eq!(
        claimed_model(&StoredPin::Other(Json::List(Vec::new()))),
        None
    );
}

#[test]
fn a_prefix_of_a_candidate_is_not_a_candidate() {
    let mut world = World::new();
    world.set_pool("MEDIUM", pool(&["model-a"]));
    world.deployment_pin = true;
    let context = world.context();
    let service = world.service(&context);
    let candidates = vec!["model-a".to_owned()];
    let selection = Selection {
        model: "model",
        candidates: &candidates,
    };
    let mut caller = request(&[(
        "metadata",
        Json::Object(request(&[("session_id", text("s1"))])),
    )]);
    let returned = runtime()
        .block_on(pin_model_for_tier(
            &service,
            &MEDIUM,
            &selection,
            &mut caller,
            None,
        ))
        .unwrap();
    assert_eq!(returned, "model");
    let key = tier_pin_cache_key(&service, &MEDIUM, "s1", None, None).unwrap();
    assert_eq!(world.claims.local.read(&key).unwrap(), None);
}

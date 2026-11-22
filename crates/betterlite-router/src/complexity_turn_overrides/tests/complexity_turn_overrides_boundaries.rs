use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::Role;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use super::support::{
    FakeEmbedder, base_settings, catalog, chat, custom_catalog, json_list, json_object, json_text,
    parts, rule, rules, semantic_settings, text_part, tier,
};
use crate::classifier_message_text::error::ClassifierTextError;
use crate::complexity_scoring::tier::Tier;
use crate::complexity_turn_overrides::error::TurnOverrideError;
use crate::complexity_turn_overrides::housekeeping::{
    cheapest_configured_tier, matched_housekeeping_sentinel,
};
use crate::complexity_turn_overrides::keywords::LexicalRules;
use crate::complexity_turn_overrides::plan::{
    apply_plan_mode_floor, matched_plan_mode_signal, plan_mode_floor, plan_mode_floor_is_top_tier,
};
use crate::complexity_turn_overrides::semantic::{SemanticError, SemanticKeywordIndex};
use crate::complexity_turn_overrides::settings::{Settings, active_tier, tier_severity};
use crate::complexity_turn_overrides::stall::detect_stalled_task;
use crate::semantic_router::service::CallerContext;
use crate::tier_definitions::model::TierSet;

fn all_pools() -> TierSet {
    catalog(&[
        ("SIMPLE", &["simple"]),
        ("MEDIUM", &["medium"]),
        ("COMPLEX", &["complex"]),
        ("REASONING", &["reasoning"]),
    ])
}

fn floor_settings(floor: &str) -> Settings {
    let mut settings = base_settings();
    settings.plan_mode_min_tier = Some(floor.to_owned());
    settings
}

#[rstest]
#[case("SIMPLE", 0)]
#[case("MEDIUM", 1)]
#[case("COMPLEX", 2)]
#[case("REASONING", 3)]
fn the_severity_positions_follow_the_built_in_order(#[case] name: &str, #[case] expected: usize) {
    let found = tier_severity(&all_pools(), &tier(name)).unwrap();
    assert_eq!(found, expected);
}

#[test]
fn a_custom_tier_set_numbers_its_definitions_in_list_order() {
    let set = custom_catalog(&["cheap", "deep"], &[("cheap", &["a"]), ("deep", &["b"])]);
    assert_eq!(
        tier_severity(&set, &Tier::Named("deep".to_owned())).unwrap(),
        1
    );
    let applied = apply_plan_mode_floor(
        &floor_settings("deep"),
        &set,
        Tier::Named("cheap".to_owned()),
    )
    .unwrap();
    assert_eq!(applied, Tier::Named("deep".to_owned()));
}

#[test]
fn an_unknown_tier_names_itself_in_the_error() {
    let floor_error = plan_mode_floor(&floor_settings("DEEP"), &all_pools()).unwrap_err();
    assert!(matches!(
        floor_error,
        TurnOverrideError::UnknownTier { ref tier } if tier.as_str() == "DEEP"
    ));
    let rule_error = LexicalRules::compile(&all_pools(), &[rule(&["x"], "DEEP")]).unwrap_err();
    assert!(matches!(
        rule_error,
        TurnOverrideError::UnknownTier { ref tier } if tier.as_str() == "DEEP"
    ));
    let custom = custom_catalog(&["cheap"], &[("cheap", &["a"])]);
    let custom_error = active_tier(&custom, "deep").unwrap_err();
    assert!(matches!(
        custom_error,
        TurnOverrideError::UnknownTier { ref tier } if tier.as_str() == "deep"
    ));
}

#[test]
fn the_top_tier_shortcut_needs_a_floor_and_a_higher_pool() {
    assert!(!plan_mode_floor_is_top_tier(&Settings::default(), &all_pools()).unwrap());
    let mut highest = all_pools();
    highest
        .pools
        .retain(|name, _pool| name == "SIMPLE" || name == "COMPLEX");
    assert!(plan_mode_floor_is_top_tier(&floor_settings("COMPLEX"), &highest).unwrap());
}

#[test]
fn the_lexical_matcher_keeps_the_first_rule_on_a_severity_tie() {
    let configured = rules(&[(&["invoice"], "COMPLEX"), (&["refund"], "COMPLEX")]);
    let compiled = LexicalRules::compile(&all_pools(), &configured).unwrap();
    let found = compiled.matches("invoice refund").unwrap();
    assert_eq!(found.matched_keyword.as_deref(), Some("invoice"));
}

#[rstest]
#[case(2, false)]
#[case(3, true)]
fn the_window_boundary_decides_how_many_repeats_count(#[case] window: u32, #[case] expected: bool) {
    let messages = vec![
        assistant_tool_use("a1", "search"),
        assistant_tool_use("a2", "lookup"),
        assistant_tool_use("a3", "search"),
    ];
    assert_eq!(detect_stalled_task(Some(&messages), window, 2), expected);
}

#[test]
fn the_repeat_threshold_is_inclusive_and_zero_is_off() {
    let messages = vec![
        assistant_tool_use("a1", "search"),
        assistant_tool_use("a2", "search"),
        assistant_tool_use("a3", "search"),
    ];
    assert!(detect_stalled_task(Some(&messages), 6, 3));
    assert!(!detect_stalled_task(Some(&messages), 6, 4));
    assert!(!detect_stalled_task(Some(&messages), 6, 0));
    assert!(!detect_stalled_task(Some(&[]), 6, 3));
}

#[test]
fn the_error_count_is_inclusive_and_needs_the_newest_call() {
    let all_errors = vec![
        tool_results(&[("a", true), ("b", true), ("c", true)]),
        assistant_tool_use("a", "one"),
        assistant_tool_use("b", "two"),
        assistant_tool_use("c", "three"),
    ];
    assert!(detect_stalled_task(Some(&all_errors), 3, 3));
    let two_errors = vec![
        tool_results(&[("a", true), ("b", false), ("c", true)]),
        assistant_tool_use("a", "one"),
        assistant_tool_use("b", "two"),
        assistant_tool_use("c", "three"),
    ];
    assert!(!detect_stalled_task(Some(&two_errors), 3, 3));
    let newest_clean = vec![
        tool_results(&[("a", true)]),
        assistant_tool_use("a", "one"),
        assistant_tool_use("b", "two"),
    ];
    assert!(!detect_stalled_task(Some(&newest_clean), 2, 2));
    let two_older_errors = vec![
        tool_results(&[("a", true), ("b", true)]),
        assistant_tool_use("a", "one"),
        assistant_tool_use("b", "two"),
        assistant_tool_use("c", "three"),
    ];
    assert!(!detect_stalled_task(Some(&two_older_errors), 3, 2));
}

#[test]
fn only_the_last_result_for_a_call_id_binds() {
    let messages = vec![
        tool_results(&[("a", true), ("a", false), ("b", true)]),
        assistant_tool_use("a", "one"),
        assistant_tool_use("b", "two"),
    ];
    assert!(!detect_stalled_task(Some(&messages), 2, 2));
    let decoy = vec![
        tool_results(&[("z", true)]),
        assistant_tool_use("a", "one"),
        assistant_tool_use("b", "two"),
    ];
    assert!(!detect_stalled_task(Some(&decoy), 2, 2));
}

#[test]
fn a_semantic_score_must_reach_the_threshold() {
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("invoice", vec![1.0, 0.0]),
        ("exact", vec![0.7, 0.7]),
        ("orthogonal", vec![0.0, 1.0]),
    ]));
    let index = SemanticKeywordIndex::new(
        &semantic_settings(&[(&["invoice"], "COMPLEX")], 1.0),
        &all_pools(),
        Arc::clone(&embedder),
    )
    .unwrap();
    let caller = CallerContext::default();
    assert_eq!(
        runtime().block_on(index.tier_override("exact", &caller)),
        Some(tier("COMPLEX"))
    );
    assert_eq!(
        runtime().block_on(index.tier_override("orthogonal", &caller)),
        None
    );
}

#[test]
fn a_route_scores_by_its_best_utterance_not_the_mean() {
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("invoice", vec![1.0, 0.0]),
        ("receipt", vec![0.0, 1.0]),
        ("exact", vec![1.0, 0.0]),
    ]));
    let index = SemanticKeywordIndex::new(
        &semantic_settings(&[(&["invoice", "receipt"], "COMPLEX")], 0.9),
        &all_pools(),
        embedder,
    )
    .unwrap();
    assert_eq!(
        runtime().block_on(index.tier_override("exact", &CallerContext::default())),
        Some(tier("COMPLEX"))
    );
}

#[test]
fn a_zero_threshold_passes_even_an_orthogonal_query() {
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("invoice", vec![1.0, 0.0]),
        ("orthogonal", vec![0.0, 1.0]),
        ("opposite", vec![-1.0, 0.0]),
    ]));
    let index = SemanticKeywordIndex::new(
        &semantic_settings(&[(&["invoice"], "COMPLEX")], 0.0),
        &all_pools(),
        embedder,
    )
    .unwrap();
    assert_eq!(
        runtime().block_on(index.tier_override("orthogonal", &CallerContext::default())),
        Some(tier("COMPLEX"))
    );
    assert_eq!(
        runtime().block_on(index.tier_override("opposite", &CallerContext::default())),
        Some(tier("COMPLEX"))
    );
}

#[test]
fn an_out_of_range_threshold_fails_index_construction() {
    let embedder = Arc::new(FakeEmbedder::new(&[("test", vec![1.0, 0.0])]));
    let low = semantic_settings(&[(&["invoice"], "COMPLEX")], -0.1);
    assert!(matches!(
        SemanticKeywordIndex::new(&low, &all_pools(), Arc::clone(&embedder)),
        Err(SemanticError::Threshold)
    ));
    let high = semantic_settings(&[(&["invoice"], "COMPLEX")], 1.5);
    assert!(matches!(
        SemanticKeywordIndex::new(&high, &all_pools(), embedder),
        Err(SemanticError::Threshold)
    ));
}

#[test]
fn a_cancelled_query_embedding_falls_back_to_no_override() {
    let embedder = Arc::new(
        FakeEmbedder::new(&[("test", vec![1.0, 0.0]), ("invoice", vec![1.0, 0.0])]).stalled_query(),
    );
    let index = SemanticKeywordIndex::new(
        &semantic_settings(&[(&["invoice"], "COMPLEX")], 0.5),
        &all_pools(),
        embedder,
    )
    .unwrap();
    let found = runtime().block_on(async {
        tokio::time::pause();
        index
            .tier_override("invoice", &CallerContext::default())
            .await
    });
    assert_eq!(found, None);
}

#[test]
fn a_classifier_text_failure_names_the_offending_part() {
    let messages = vec![parts(
        Role::User,
        vec![text_part(Json::Number(serde_json::Number::from(7)))],
    )];
    let error =
        matched_plan_mode_signal(&floor_settings("COMPLEX"), None, Some(&messages)).unwrap_err();
    assert!(matches!(
        error,
        ClassifierTextError::PartTextNotString { part: 0 }
    ));
}

#[test]
fn empty_inputs_stay_inert() {
    let settings = Settings::default();
    assert_eq!(matched_housekeeping_sentinel(&settings, Some("")), None);
    assert_eq!(matched_housekeeping_sentinel(&settings, None), None);
    assert_eq!(
        cheapest_configured_tier(&catalog(&[("SIMPLE", &[])])).unwrap(),
        None
    );
    let messages = vec![chat(Role::User, "hi")];
    assert!(!detect_stalled_task(Some(&messages), 6, 3));
}

#[test]
fn a_plan_signal_is_not_required_when_the_floor_is_unset() {
    let body = json_object(&[(
        "system",
        json_text("You are currently running in \"Plan\" mode."),
    )]);
    let kwargs = super::support::request_kwargs(body);
    assert_eq!(
        matched_plan_mode_signal(&Settings::default(), Some(&kwargs), None).unwrap(),
        None
    );
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime starts")
}

fn assistant_tool_use(id: &str, name: &str) -> Json {
    json_object(&[
        ("role", json_text("assistant")),
        (
            "content",
            json_list(vec![json_object(&[
                ("type", json_text("tool_use")),
                ("id", json_text(id)),
                ("name", json_text(name)),
                ("input", Json::Null),
            ])]),
        ),
    ])
}

fn tool_results(results: &[(&str, bool)]) -> Json {
    json_object(&[
        ("role", json_text("user")),
        (
            "content",
            json_list(
                results
                    .iter()
                    .map(|(id, is_error)| {
                        json_object(&[
                            ("type", json_text("tool_result")),
                            ("tool_use_id", json_text(id)),
                            ("is_error", Json::Bool(*is_error)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, Ready, ready};

use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    SUPPORTS_SEED, SUPPORTS_TEMPERATURE, cost_map, deployment, group, info, object, priority_group,
    raw, text, with_args, with_priorities,
};
use crate::router_predicates::JsonMap;
use crate::routing_groups::effort::{
    GroupReasoningEfforts, intersect_supported_efforts, nearest_declared_effort,
    resolve_supported_efforts,
};
use crate::routing_groups::group::{
    MAX_MODEL_PRIORITY, RoutingGroup, parse_routing_groups, validate_routing_strategy,
};
use crate::routing_groups::index::RoutingGroupIndex;
use crate::routing_groups::params::{
    drop_classifier_effort, is_classifier_internal_call, params_with_reasoning_override,
    tier_params_the_target_accepts,
};
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroups};
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::{RoutingStrategy, VALID_ROUTING_STRATEGIES};

fn settled(case: fn()) -> Ready<()> {
    case();
    ready(())
}

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn parse_error(groups: &[JsonMap], known: &[&str]) -> String {
    let model_names: BTreeSet<String> = known.iter().map(|name| (*name).to_owned()).collect();
    parse_routing_groups(Some(groups), &model_names)
        .unwrap_err()
        .to_string()
}

fn built(groups: Vec<RoutingGroup>, known: &[&str], aliases: &[&str]) -> RoutingGroupIndex {
    let model_names: BTreeSet<String> = known.iter().map(|name| (*name).to_owned()).collect();
    let alias_map: BTreeMap<String, String> = aliases
        .iter()
        .map(|name| ((*name).to_owned(), (*name).to_owned()))
        .collect();
    RoutingGroupIndex::build(groups, &model_names, &alias_map)
        .unwrap()
        .index
}

fn an_empty_name_is_rejected() {
    assert_eq!(
        parse_error(&[raw("", &["a"], "simple-shuffle")], &[]),
        "routing_groups: group_name must be non-empty."
    );
}

fn the_default_name_is_reserved() {
    assert_eq!(
        parse_error(&[raw("default", &["a"], "simple-shuffle")], &[]),
        "routing_groups: 'default' is reserved for the implicit fallback group."
    );
}

fn duplicate_names_report_the_smallest() {
    assert_eq!(
        parse_error(
            &[
                raw("b", &["a"], "simple-shuffle"),
                raw("a", &["b"], "simple-shuffle"),
                raw("a", &["c"], "simple-shuffle"),
                raw("b", &["d"], "simple-shuffle"),
            ],
            &[],
        ),
        "routing_groups: group names must be unique, duplicate group_name 'a'."
    );
}

fn an_unknown_strategy_names_itself() {
    assert_eq!(
        parse_error(&[raw("cheap", &["a"], "nope")], &[]),
        "Invalid routing_strategy: 'nope'. Valid options: ['simple-shuffle', 'lar1', 'least-busy', 'latency-based-routing', 'cost-based-routing', 'usage-based-routing-v2', 'usage-based-routing', 'provider-budget-routing']. Check 'router_settings.routing_strategy' in your config.yaml or the 'routing_strategy' parameter if using the Router SDK directly."
    );
}

fn every_valid_strategy_is_accepted() {
    validate_routing_strategy(None).unwrap();
    for strategy in VALID_ROUTING_STRATEGIES {
        assert!(
            validate_routing_strategy(Some(strategy.wire_name())).is_ok(),
            "{strategy:?}"
        );
    }
    assert!(validate_routing_strategy(Some("nope")).is_err());
}

fn priorities_require_the_priority_strategy() {
    let mut raw = raw("cheap", &["a"], "simple-shuffle");
    raw.insert(
        "model_priorities".to_owned(),
        Json::Object(info(&[("a", number(1))])),
    );
    assert_eq!(
        parse_error(&[raw], &[]),
        "model_priorities requires routing_strategy='priority'"
    );
}

fn empty_priorities_pass_a_strategy_group() {
    let mut raw = raw("cheap", &["a"], "simple-shuffle");
    raw.insert("model_priorities".to_owned(), Json::Object(JsonMap::new()));
    let parsed = parse_routing_groups(Some(&[raw]), &BTreeSet::new()).unwrap();
    assert_eq!(parsed.groups[0].model_priorities, Some(BTreeMap::new()));
}

fn an_empty_priority_group_is_rejected() {
    assert_eq!(
        parse_error(&[with_priorities(raw("tiers", &[], "priority"), &[])], &[]),
        "Priority routing groups require nonempty, distinct models"
    );
}

fn duplicate_priority_members_are_rejected() {
    assert_eq!(
        parse_error(
            &[with_priorities(
                raw("tiers", &["a", "a"], "priority"),
                &[("a", number(1))]
            )],
            &[],
        ),
        "Priority routing groups require nonempty, distinct models"
    );
}

fn a_priority_group_needs_priorities() {
    assert_eq!(
        parse_error(&[raw("tiers", &["a"], "priority")], &[]),
        "model_priorities must contain exactly the group's models"
    );
}

fn priorities_must_cover_exactly_the_members() {
    assert_eq!(
        parse_error(
            &[with_priorities(
                raw("tiers", &["a"], "priority"),
                &[("a", number(1)), ("b", number(2))],
            )],
            &[],
        ),
        "model_priorities must contain exactly the group's models"
    );
    assert_eq!(
        parse_error(
            &[with_priorities(
                raw("tiers", &["a", "b"], "priority"),
                &[("a", number(1))]
            )],
            &[],
        ),
        "model_priorities must contain exactly the group's models"
    );
}

fn a_priority_group_checks_its_strategy_args() {
    let rejected = with_args(
        with_priorities(raw("tiers", &["a"], "priority"), &[("a", number(1))]),
        &[("ttl", number(5))],
    );
    assert_eq!(
        parse_error(&[rejected], &[]),
        "Priority routing groups use model_priorities, not routing_strategy_args"
    );
    let mut groups = parse_routing_groups(
        Some(&[with_args(
            with_priorities(raw("tiers", &["a"], "priority"), &[("a", number(1))]),
            &[],
        )]),
        &BTreeSet::new(),
    )
    .unwrap()
    .groups;
    assert_eq!(groups.remove(0).routing_strategy, GroupStrategy::Priority);
}

fn a_model_in_two_groups_names_both() {
    assert_eq!(
        parse_error(
            &[
                raw("one", &["a"], "simple-shuffle"),
                raw("two", &["a"], "simple-shuffle"),
            ],
            &[],
        ),
        "routing_groups: model_name 'a' appears in 'one' and 'two'. Each model may belong to at most one group."
    );
}

fn every_owner_is_named() {
    assert_eq!(
        parse_error(
            &[
                raw("one", &["a"], "simple-shuffle"),
                raw("two", &["a"], "simple-shuffle"),
                raw("three", &["a"], "simple-shuffle"),
            ],
            &[],
        ),
        "routing_groups: model_name 'a' appears in 'one' and 'two' and 'three'. Each model may belong to at most one group."
    );
}

fn conflicts_join_with_semicolons() {
    assert_eq!(
        parse_error(
            &[
                raw("one", &["a", "b"], "simple-shuffle"),
                raw("two", &["a", "b"], "simple-shuffle"),
            ],
            &[],
        ),
        "routing_groups: model_name 'a' appears in 'one' and 'two'; model_name 'b' appears in 'one' and 'two'. Each model may belong to at most one group."
    );
}

fn a_priority_member_does_not_conflict() {
    let parsed = parse_routing_groups(
        Some(&[
            raw("one", &["a"], "simple-shuffle"),
            with_priorities(raw("tiers", &["a"], "priority"), &[("a", number(1))]),
        ]),
        &BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(parsed.groups.len(), 2);
}

fn priority_bounds_are_accepted() {
    let parsed = parse_routing_groups(
        Some(&[
            with_priorities(raw("one", &["a"], "priority"), &[("a", number(1))]),
            with_priorities(
                raw("top", &["b"], "priority"),
                &[("b", number(MAX_MODEL_PRIORITY))],
            ),
        ]),
        &BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(
        parsed.groups[1].model_priorities,
        Some(BTreeMap::from([(String::from("b"), MAX_MODEL_PRIORITY)]))
    );
}

fn a_priority_outside_the_bounds_is_rejected() {
    let expected =
        "routing_groups.model_priorities['a'] must be an integer from 1 to 9007199254740991";
    for value in [
        number(0),
        number(MAX_MODEL_PRIORITY + 1),
        Json::Number(Number::from_f64(1.5).unwrap()),
        Json::Number(Number::from(-1_i64)),
        Json::Bool(true),
        text("2"),
    ] {
        assert_eq!(
            parse_error(
                &[with_priorities(
                    raw("tiers", &["a"], "priority"),
                    &[("a", value)]
                )],
                &[]
            ),
            expected
        );
    }
}

fn field_types_name_the_field_and_the_expected_shape() {
    let mut missing_name = raw("cheap", &["a"], "simple-shuffle");
    drop(missing_name.shift_remove("group_name"));
    assert_eq!(
        parse_error(&[missing_name], &[]),
        "routing_groups.group_name must be a string"
    );
    let mut bad_models = raw("cheap", &["a"], "simple-shuffle");
    bad_models.insert("models".to_owned(), text("a"));
    assert_eq!(
        parse_error(&[bad_models], &[]),
        "routing_groups.models must be a list of strings"
    );
    let mut bad_member = raw("cheap", &["a"], "simple-shuffle");
    bad_member.insert("models".to_owned(), Json::List(vec![number(1)]));
    assert_eq!(
        parse_error(&[bad_member], &[]),
        "routing_groups.models must be a list of strings"
    );
    let mut bad_args = raw("cheap", &["a"], "simple-shuffle");
    bad_args.insert("routing_strategy_args".to_owned(), text("ttl"));
    assert_eq!(
        parse_error(&[bad_args], &[]),
        "routing_groups.routing_strategy_args must be an object"
    );
    let mut bad_priorities = raw("cheap", &["a"], "simple-shuffle");
    bad_priorities.insert("model_priorities".to_owned(), Json::List(Vec::new()));
    assert_eq!(
        parse_error(&[bad_priorities], &[]),
        "routing_groups.model_priorities must be an object of integers"
    );
}

fn a_priority_group_name_may_not_shadow_a_model() {
    let known: BTreeSet<String> = BTreeSet::from([String::from("tiers")]);
    let aliases = BTreeMap::new();
    let error =
        RoutingGroupIndex::build(vec![priority_group("tiers", &[("a", 1)])], &known, &aliases)
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Priority routing group names must not shadow a model or alias"
    );
}

fn a_shadowing_strategy_group_warns_and_keeps_its_members() {
    let known: BTreeSet<String> = BTreeSet::from([String::from("cheap")]);
    let aliases = BTreeMap::new();
    let built = RoutingGroupIndex::build(
        vec![group("cheap", &["a"], "simple-shuffle")],
        &known,
        &aliases,
    )
    .unwrap();
    assert_eq!(
        built.warnings,
        vec![String::from(
            "routing_groups: group_name 'cheap' is shadowed by an existing model_name or model_group_alias; the group's strategy still applies to its members, but the name is not callable until renamed."
        )]
    );
    assert!(built.index.callable_group("cheap").is_none());
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::SimpleShuffle, None).unwrap();
    let groups: RoutingGroups = built
        .index
        .dispatch_groups(&mut dispatch, &|_| None)
        .unwrap();
    assert_eq!(
        groups
            .route("a", &BTreeSet::new())
            .map(|route| route.name.as_str()),
        Some("cheap")
    );
}

fn an_alias_shadows_a_group_just_the_same() {
    let index = built(
        vec![group("cheap", &["a"], "simple-shuffle")],
        &[],
        &["cheap"],
    );
    assert!(index.callable_group("cheap").is_none());
    assert!(!index.is_priority_group("cheap"));
}

fn an_alias_resolves_before_the_priority_and_alternatives_lookups() {
    let mut aliases = BTreeMap::new();
    aliases.insert(String::from("premium"), String::from("escalation"));
    let known = BTreeSet::new();
    let index = RoutingGroupIndex::build(
        vec![priority_group("escalation", &[("a", 1), ("b", 2)])],
        &known,
        &aliases,
    )
    .unwrap()
    .index;
    assert!(index.callable_group("premium").is_none());
    assert!(index.is_priority_group("premium"));
    assert!(index.is_priority_group("escalation"));
    assert!(index.has_alternatives("premium", &|_| 1));
    assert!(!index.has_alternatives("premium", &|member| usize::from(member == "a")));
    assert!(!index.is_priority_group("unknown"));
}

fn a_duplicate_group_in_the_index_names_the_group() {
    let known = BTreeSet::new();
    let aliases = BTreeMap::new();
    let error = RoutingGroupIndex::build(
        vec![
            group("cheap", &["a"], "simple-shuffle"),
            group("cheap", &["b"], "simple-shuffle"),
        ],
        &known,
        &aliases,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "routing_groups: group names must be unique, duplicate group_name 'cheap'."
    );
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
    let map = cost_map(&[]);
    let model_info = info(&[
        ("supports_reasoning", Json::Bool(true)),
        ("reasoning_effort_levels", Json::List(Vec::new())),
    ]);
    assert_eq!(
        resolve_supported_efforts(&model_info, true, &map),
        Some(Vec::new())
    );
}

fn a_declared_value_that_is_not_a_list_falls_back_to_the_flags() {
    let map = cost_map(&[]);
    let model_info = info(&[
        ("supports_reasoning", Json::Bool(true)),
        ("reasoning_effort_levels", text("high")),
    ]);
    assert_eq!(resolve_supported_efforts(&model_info, false, &map), None);
}

fn a_flag_that_is_not_a_boolean_still_advertises() {
    let map = cost_map(&[]);
    let model_info = info(&[
        ("supports_reasoning", Json::Bool(true)),
        ("supports_minimal_reasoning_effort", text("yes")),
    ]);
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

fn an_extra_body_that_empties_is_removed() {
    let deployment_params = info(&[("extra_body", object(&[("reasoning_effort", text("low"))]))]);
    let request = info(&[("reasoning_effort", text("high"))]);
    assert_eq!(
        params_with_reasoning_override(&deployment_params, &request),
        JsonMap::new()
    );
}

fn a_non_object_extra_body_is_left_alone() {
    let deployment_params = info(&[("extra_body", text("keep"))]);
    let request = info(&[("reasoning_effort", text("high"))]);
    assert_eq!(
        params_with_reasoning_override(&deployment_params, &request),
        deployment_params
    );
}

fn only_an_auto_router_classifier_call_names_itself() {
    assert!(!is_classifier_internal_call(&info(&[(
        "litellm_metadata",
        text("x")
    )])));
    assert!(!is_classifier_internal_call(&info(&[(
        "metadata",
        Json::Null
    )])));
}

fn a_null_classifier_effort_is_not_dropped() {
    let mut kwargs = info(&[
        ("reasoning_effort", Json::Null),
        (
            "metadata",
            object(&[("internal_call_origin", text("autorouter_classifier"))]),
        ),
    ]);
    assert_eq!(
        drop_classifier_effort(&deployment("m", "m1"), "m", &mut kwargs, &SUPPORTS_SEED),
        None
    );
    assert_eq!(kwargs.get("reasoning_effort"), Some(&Json::Null));
}

fn a_non_object_body_keeps_the_drop_to_the_top_level() {
    let mut kwargs = info(&[
        ("reasoning_effort", text("high")),
        (
            "metadata",
            object(&[("internal_call_origin", text("autorouter_classifier"))]),
        ),
        ("proxy_server_request", text("kept")),
    ]);
    assert!(
        drop_classifier_effort(&deployment("m", "m1"), "m", &mut kwargs, &SUPPORTS_SEED).is_some()
    );
    assert_eq!(kwargs.get("reasoning_effort"), None);
    assert_eq!(kwargs.get("proxy_server_request"), Some(&text("kept")));
}

fn an_empty_tier_has_nothing_to_drop() {
    let accepted = tier_params_the_target_accepts(
        "m",
        &JsonMap::new(),
        &JsonMap::new(),
        &[deployment("m", "m1")],
        &SUPPORTS_SEED,
    );
    assert_eq!(accepted.params, JsonMap::new());
    assert!(accepted.dropped.is_empty());
}

fn a_tier_whose_params_all_drop_keeps_none() {
    let tier = info(&[("temperature", number(1)), ("top_p", number(1))]);
    let accepted = tier_params_the_target_accepts(
        "m",
        &tier,
        &JsonMap::new(),
        &[deployment("m", "m1")],
        &SUPPORTS_SEED,
    );
    assert_eq!(accepted.params, JsonMap::new());
    assert_eq!(
        accepted.dropped,
        BTreeSet::from([String::from("temperature"), String::from("top_p")])
    );
}

fn a_kept_tier_param_keeps_its_value() {
    let tier = info(&[("top_p", number(1)), ("temperature", number(2))]);
    let accepted = tier_params_the_target_accepts(
        "m",
        &tier,
        &JsonMap::new(),
        &[deployment("m", "m1")],
        &SUPPORTS_TEMPERATURE,
    );
    assert_eq!(accepted.params, info(&[("temperature", number(2))]));
    assert_eq!(accepted.dropped, BTreeSet::from([String::from("top_p")]));
}

#[rstest]
#[case::an_empty_name_is_rejected(settled(an_empty_name_is_rejected))]
#[case::the_default_name_is_reserved(settled(the_default_name_is_reserved))]
#[case::duplicate_names_report_the_smallest(settled(duplicate_names_report_the_smallest))]
#[case::an_unknown_strategy_names_itself(settled(an_unknown_strategy_names_itself))]
#[case::every_valid_strategy_is_accepted(settled(every_valid_strategy_is_accepted))]
#[case::priorities_require_the_priority_strategy(settled(
    priorities_require_the_priority_strategy
))]
#[case::empty_priorities_pass_a_strategy_group(settled(empty_priorities_pass_a_strategy_group))]
#[case::an_empty_priority_group_is_rejected(settled(an_empty_priority_group_is_rejected))]
#[case::duplicate_priority_members_are_rejected(settled(duplicate_priority_members_are_rejected))]
#[case::a_priority_group_needs_priorities(settled(a_priority_group_needs_priorities))]
#[case::priorities_must_cover_exactly_the_members(settled(
    priorities_must_cover_exactly_the_members
))]
#[case::a_priority_group_checks_its_strategy_args(settled(
    a_priority_group_checks_its_strategy_args
))]
#[case::a_model_in_two_groups_names_both(settled(a_model_in_two_groups_names_both))]
#[case::every_owner_is_named(settled(every_owner_is_named))]
#[case::conflicts_join_with_semicolons(settled(conflicts_join_with_semicolons))]
#[case::a_priority_member_does_not_conflict(settled(a_priority_member_does_not_conflict))]
#[case::priority_bounds_are_accepted(settled(priority_bounds_are_accepted))]
#[case::a_priority_outside_the_bounds_is_rejected(settled(
    a_priority_outside_the_bounds_is_rejected
))]
#[case::field_types_name_the_field_and_the_expected_shape(settled(
    field_types_name_the_field_and_the_expected_shape
))]
#[case::a_priority_group_name_may_not_shadow_a_model(settled(
    a_priority_group_name_may_not_shadow_a_model
))]
#[case::a_shadowing_strategy_group_warns_and_keeps_its_members(settled(
    a_shadowing_strategy_group_warns_and_keeps_its_members
))]
#[case::an_alias_shadows_a_group_just_the_same(settled(an_alias_shadows_a_group_just_the_same))]
#[case::an_alias_resolves_before_the_priority_and_alternatives_lookups(settled(
    an_alias_resolves_before_the_priority_and_alternatives_lookups
))]
#[case::a_duplicate_group_in_the_index_names_the_group(settled(
    a_duplicate_group_in_the_index_names_the_group
))]
#[case::an_empty_declared_list_keeps_the_request(settled(
    an_empty_declared_list_keeps_the_request
))]
#[case::an_empty_side_empties_the_intersection(settled(an_empty_side_empties_the_intersection))]
#[case::a_late_unknown_deployment_clears_the_aggregate(settled(
    a_late_unknown_deployment_clears_the_aggregate
))]
#[case::an_empty_declared_list_is_a_known_empty(settled(an_empty_declared_list_is_a_known_empty))]
#[case::a_declared_value_that_is_not_a_list_falls_back_to_the_flags(settled(
    a_declared_value_that_is_not_a_list_falls_back_to_the_flags
))]
#[case::a_flag_that_is_not_a_boolean_still_advertises(settled(
    a_flag_that_is_not_a_boolean_still_advertises
))]
#[case::an_extra_body_that_empties_is_removed(settled(an_extra_body_that_empties_is_removed))]
#[case::a_non_object_extra_body_is_left_alone(settled(a_non_object_extra_body_is_left_alone))]
#[case::only_an_auto_router_classifier_call_names_itself(settled(
    only_an_auto_router_classifier_call_names_itself
))]
#[case::a_null_classifier_effort_is_not_dropped(settled(a_null_classifier_effort_is_not_dropped))]
#[case::a_non_object_body_keeps_the_drop_to_the_top_level(settled(
    a_non_object_body_keeps_the_drop_to_the_top_level
))]
#[case::an_empty_tier_has_nothing_to_drop(settled(an_empty_tier_has_nothing_to_drop))]
#[case::a_tier_whose_params_all_drop_keeps_none(settled(a_tier_whose_params_all_drop_keeps_none))]
#[case::a_kept_tier_param_keeps_its_value(settled(a_kept_tier_param_keeps_its_value))]
#[tokio::test]
async fn routing_groups_boundaries<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    case.await;
}

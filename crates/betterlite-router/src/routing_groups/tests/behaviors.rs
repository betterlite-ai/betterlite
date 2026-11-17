use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, Ready, ready};

use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    GroupServices, Supports, Unresolved, cost_map, deployment, group, info, object, priority_group,
    raw, raw_priority, text,
};
use crate::context::{self, RouteStage};
use crate::router_core::candidates::CandidateSet;
use crate::router_core::tests::support::{NoLimits, Providers, route_context, route_request};
use crate::router_predicates::JsonMap;
use crate::routing_groups::effort::declared::{
    ADVERTISEMENT_ORDER, EffortFlag, declared_efforts_for_model, effort_from_wire, effort_wire_name,
};
use crate::routing_groups::effort::{
    GroupReasoningEfforts, deployment_is_catalog_mapped, intersect_supported_efforts,
    nearest_declared_effort, resolve_supported_efforts, supports_none_reasoning_effort,
};
use crate::routing_groups::group::{RoutingGroup, parse_routing_groups};
use crate::routing_groups::index::RoutingGroupIndex;
use crate::routing_groups::params::{
    drop_classifier_effort, is_classifier_internal_call, params_with_reasoning_override,
    tier_params_the_target_accepts,
};
use crate::routing_groups::priority::{group_row, prioritized};
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroups};
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;

fn settled(case: fn()) -> Ready<()> {
    case();
    ready(())
}

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn parsed(groups: &[JsonMap], known: &[&str]) -> Vec<RoutingGroup> {
    let known: BTreeSet<String> = known.iter().map(|name| (*name).to_owned()).collect();
    parse_routing_groups(Some(groups), &known).unwrap().groups
}

fn built(groups: Vec<RoutingGroup>, known: &[&str], aliases: &[&str]) -> RoutingGroupIndex {
    let known: BTreeSet<String> = known.iter().map(|name| (*name).to_owned()).collect();
    let aliases: BTreeSet<String> = aliases.iter().map(|name| (*name).to_owned()).collect();
    RoutingGroupIndex::build(groups, &known, &aliases)
        .unwrap()
        .index
}

fn resolve_info(entries: &[(&str, Json)], mapped: bool) -> Option<Vec<ReasoningEffort>> {
    let map = cost_map(&[]);
    resolve_supported_efforts(&info(entries), mapped, &map)
}

fn parse_reads_a_strategy_group() {
    let groups = parsed(&[raw("cheap", &["a", "b"], "least-busy")], &["a", "b"]);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group_name, "cheap");
    assert_eq!(groups[0].models, vec![String::from("a"), String::from("b")]);
    assert_eq!(
        groups[0].routing_strategy,
        GroupStrategy::Routing(RoutingStrategy::LeastBusy)
    );
    assert_eq!(groups[0].model_priorities, None);
}

fn parse_reads_a_priority_group() {
    let groups = parsed(
        &[raw_priority(
            "tiers",
            &["a", "b"],
            &[("a", number(2)), ("b", number(1))],
        )],
        &[],
    );
    assert_eq!(groups[0].routing_strategy, GroupStrategy::Priority);
    assert_eq!(
        groups[0].model_priorities,
        Some(BTreeMap::from([
            (String::from("a"), 2),
            (String::from("b"), 1)
        ]))
    );
}

fn unknown_members_warn_once_each() {
    let known: BTreeSet<String> = BTreeSet::from([String::from("a"), String::from("b")]);
    let outcome = parse_routing_groups(
        Some(&[
            raw("cheap", &["a", "missing"], "simple-shuffle"),
            raw("rich", &["b", "gone"], "least-busy"),
        ]),
        &known,
    )
    .unwrap();
    assert_eq!(
        outcome.warnings,
        vec![
            String::from(
                "routing_groups: model_name 'missing' (group 'cheap') is not in model_list; the group entry will only take effect once a deployment with that model_name is added."
            ),
            String::from(
                "routing_groups: model_name 'gone' (group 'rich') is not in model_list; the group entry will only take effect once a deployment with that model_name is added."
            ),
        ]
    );
}

fn without_known_models_nothing_warns() {
    let outcome = parse_routing_groups(
        Some(&[raw("cheap", &["missing"], "simple-shuffle")]),
        &BTreeSet::new(),
    )
    .unwrap();
    assert!(outcome.warnings.is_empty());
}

fn no_groups_parse_to_nothing() {
    let outcome = parse_routing_groups(None, &BTreeSet::from([String::from("a")])).unwrap();
    assert!(outcome.groups.is_empty());
    assert!(outcome.warnings.is_empty());
}

fn priority_order_is_written_on_the_member() {
    let groups = parsed(&[raw_priority("tiers", &["a"], &[("a", number(7))])], &[]);
    let deployment = deployment("a", "a1");
    let ordered = prioritized(&groups[0], &deployment);
    assert_eq!(ordered.litellm_params.get("order"), Some(&number(7)));
    assert_eq!(deployment.litellm_params.get("order"), None);
}

fn other_groups_keep_their_deployments() {
    let groups = parsed(&[raw("cheap", &["a"], "least-busy")], &[]);
    let deployment = deployment("a", "a1");
    assert_eq!(prioritized(&groups[0], &deployment), deployment);
    assert_eq!(&group_row("cheap", &deployment).model_name, "cheap");
}

fn rows_rename_and_strip_access_groups() {
    let index = built(vec![group("cheap", &["a"], "least-busy")], &["a"], &[]);
    let mut deployment = deployment("a", "a1");
    deployment
        .model_info
        .insert("access_groups".to_owned(), Json::List(vec![text("g1")]));
    let rows = index.rows(Some("cheap"), &|member| {
        if member == "a" {
            vec![deployment.clone()]
        } else {
            Vec::new()
        }
    });
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model_name, "cheap");
    assert_eq!(rows[0].model_info.get("access_groups"), None);
    assert_eq!(
        rows[0].litellm_params.get("model"),
        deployment.litellm_params.get("model")
    );
}

fn rows_cover_every_callable_group() {
    let index = built(
        vec![
            group("cheap", &["a"], "simple-shuffle"),
            group("rich", &["b"], "simple-shuffle"),
        ],
        &["a", "b"],
        &[],
    );
    let rows = index.rows(None, &|member| vec![deployment(member, member)]);
    let names: Vec<&str> = rows.iter().map(|row| row.model_name.as_str()).collect();
    assert_eq!(names, vec!["cheap", "rich"]);
}

fn a_shadowed_group_is_not_callable() {
    let known: BTreeSet<String> = BTreeSet::from([String::from("cheap")]);
    let aliases = BTreeSet::new();
    let built = RoutingGroupIndex::build(
        vec![group("cheap", &["a"], "simple-shuffle")],
        &known,
        &aliases,
    )
    .unwrap();
    assert!(built.index.callable_group("cheap").is_none());
    assert!(!built.index.is_priority_group("cheap"));
    assert!(built.index.rows(Some("cheap"), &|_| Vec::new()).is_empty());
    assert_eq!(built.warnings.len(), 1);
    assert!(
        built.warnings[0].contains("is shadowed by an existing model_name or model_group_alias")
    );
}

fn model_membership_feeds_the_selector_scope() {
    let index = built(
        vec![
            group("cheap", &["a"], "least-busy"),
            priority_group("tiers", &[("b", 1)]),
        ],
        &["a", "b"],
        &[],
    );
    assert_eq!(
        index
            .claiming_group("a")
            .map(|group| group.group_name.as_str()),
        Some("cheap")
    );
    assert_eq!(index.claiming_group("b"), None);
}

fn dispatch_groups_carry_strategies_and_selectors() {
    let index = built(
        vec![
            group("cheap", &["a"], "least-busy"),
            priority_group("tiers", &[("b", 1)]),
        ],
        &["a", "b"],
        &[],
    );
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::SimpleShuffle, None).unwrap();
    let groups: RoutingGroups = index.dispatch_groups(&mut dispatch).unwrap();
    let shadowing = BTreeSet::new();
    let cheap = groups.route("cheap", &shadowing).unwrap();
    assert_eq!(
        cheap.strategy,
        GroupStrategy::Routing(RoutingStrategy::LeastBusy)
    );
    assert!(cheap.selector.is_some());
    let tiers = groups.route("tiers", &shadowing).unwrap();
    assert_eq!(tiers.strategy, GroupStrategy::Priority);
    assert_eq!(tiers.selector, None);
    assert_eq!(groups.route("unknown", &shadowing), None);
}

fn alternatives_count_member_deployments() {
    let index = built(
        vec![group("cheap", &["a", "b"], "simple-shuffle")],
        &["a", "b"],
        &[],
    );
    assert!(index.has_alternatives("cheap", &|_member| 2));
    assert!(!index.has_alternatives("cheap", &|member| usize::from(member == "a")));
    assert!(!index.has_alternatives("unknown", &|_member| 2));
}

fn declared_levels_win_over_flags() {
    let map = cost_map(&[]);
    let model_info = info(&[
        ("supports_reasoning", Json::Bool(true)),
        ("supports_xhigh_reasoning_effort", Json::Bool(true)),
        (
            "reasoning_effort_levels",
            Json::List(vec![text("max"), text("low"), text("bogus")]),
        ),
    ]);
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(vec![ReasoningEffort::Low, ReasoningEffort::Max])
    );
}

fn flags_advertise_the_polarity_of_each_level() {
    let map = cost_map(&[]);
    let model_info = info(&[
        ("supports_reasoning", Json::Bool(true)),
        ("supports_low_reasoning_effort", Json::Bool(false)),
        ("supports_xhigh_reasoning_effort", Json::Bool(true)),
    ]);
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
    assert_eq!(
        resolve_info(&[("key", text("openai/x"))], true),
        Some(Vec::new())
    );
    assert_eq!(resolve_info(&[("key", text("openai/x"))], false), None);
}

fn explicit_false_empties_even_with_levels() {
    assert_eq!(
        resolve_info(
            &[
                ("supports_reasoning", Json::Bool(false)),
                ("supports_low_reasoning_effort", Json::Bool(true)),
            ],
            false
        ),
        Some(Vec::new())
    );
}

fn the_bare_twin_supplies_the_flags() {
    let map = cost_map(&[("gpt-5-mini", r#"{"supports_low_reasoning_effort": false}"#)]);
    let model_info = info(&[
        ("key", text("azure/gpt-5-mini")),
        ("litellm_provider", text("azure")),
        ("supports_reasoning", Json::Bool(true)),
    ]);
    assert_eq!(
        resolve_supported_efforts(&model_info, false, &map),
        Some(vec![
            ReasoningEffort::Minimal,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ])
    );
}

fn the_own_flag_wins_over_the_bare_twin() {
    let map = cost_map(&[(
        "gpt-5-mini",
        r#"{"supports_low_reasoning_effort": false, "supports_xhigh_reasoning_effort": true}"#,
    )]);
    let model_info = info(&[
        ("key", text("azure/gpt-5-mini")),
        ("litellm_provider", text("azure")),
        ("supports_reasoning", Json::Bool(true)),
        ("supports_low_reasoning_effort", Json::Bool(true)),
        ("supports_xhigh_reasoning_effort", Json::Bool(false)),
    ]);
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
    let map = cost_map(&[
        ("openai/m", r#"{"reasoning_effort_levels": ["high"]}"#),
        ("m", r#"{"reasoning_effort_levels": ["low"]}"#),
        ("openai/e", "{}"),
        ("e", r#"{"reasoning_effort_levels": ["max"]}"#),
        ("openai/s", r#""nope""#),
    ]);
    assert_eq!(
        declared_efforts_for_model("m", "openai", &map),
        Some(vec![ReasoningEffort::High])
    );
    assert_eq!(
        declared_efforts_for_model("e", "openai", &map),
        Some(vec![ReasoningEffort::Max])
    );
    assert_eq!(declared_efforts_for_model("s", "openai", &map), None);
    assert_eq!(declared_efforts_for_model("absent", "openai", &map), None);
}

fn intersection_keeps_the_advertisement_order() {
    assert_eq!(
        intersect_supported_efforts(
            Some(&[ReasoningEffort::High, ReasoningEffort::Low]),
            Some(&[ReasoningEffort::Low, ReasoningEffort::Max]),
        ),
        Some(vec![ReasoningEffort::Low])
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
    assert_eq!(
        efforts.supported(),
        Some([ReasoningEffort::High].as_slice())
    );
    assert!(!efforts.is_unknown());
}

fn the_group_aggregate_clears_on_an_unknown_deployment() {
    let mut efforts = GroupReasoningEfforts::default();
    efforts.observe(None);
    efforts.observe(Some(vec![ReasoningEffort::Low]));
    assert_eq!(efforts.supported(), None);
    assert!(efforts.is_unknown());
}

fn catalog_mapping_needs_both_modes() {
    let mode = info(&[("mode", text("chat"))]);
    let empty = JsonMap::new();
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
    let gpt = info(&[
        ("key", text("azure/gpt-5")),
        ("litellm_provider", text("azure")),
    ]);
    assert!(!supports_none_reasoning_effort(&gpt, EffortFlag::Absent));
    assert!(supports_none_reasoning_effort(&gpt, EffortFlag::True));
    assert!(!supports_none_reasoning_effort(&gpt, EffortFlag::False));
    let chat = info(&[
        ("key", text("azure/gpt-5-chat")),
        ("litellm_provider", text("azure")),
    ]);
    assert!(supports_none_reasoning_effort(&chat, EffortFlag::Absent));
    let other = info(&[
        ("key", text("openai/gpt-5")),
        ("litellm_provider", text("openai")),
    ]);
    assert!(supports_none_reasoning_effort(&other, EffortFlag::Absent));
    assert!(!supports_none_reasoning_effort(&other, EffortFlag::False));
    let unnamed = info(&[("litellm_provider", text("azure"))]);
    assert!(supports_none_reasoning_effort(&unnamed, EffortFlag::Absent));
}

fn effort_wire_names_round_trip() {
    for effort in ADVERTISEMENT_ORDER {
        assert_eq!(effort_from_wire(effort_wire_name(effort)), Some(effort));
    }
    assert_eq!(effort_from_wire("bogus"), None);
}

fn a_request_effort_drops_every_native_carrier() {
    let deployment_params = info(&[
        ("thinking", object(&[("type", text("enabled"))])),
        (
            "output_config",
            object(&[("effort", text("low")), ("other", Json::Bool(true))]),
        ),
        ("reasoning", object(&[("effort", text("low"))])),
        (
            "extra_body",
            object(&[
                ("reasoning_effort", text("low")),
                ("thinking", object(&[("type", text("enabled"))])),
                ("temperature", number(1)),
                ("output_config", object(&[("effort", text("low"))])),
            ]),
        ),
    ]);
    let request = info(&[("reasoning_effort", text("high"))]);
    assert_eq!(
        params_with_reasoning_override(&deployment_params, &request),
        info(&[
            ("output_config", object(&[("other", Json::Bool(true))])),
            ("extra_body", object(&[("temperature", number(1))])),
        ])
    );
}

fn without_a_request_effort_every_carrier_stays() {
    let deployment_params = info(&[
        ("thinking", object(&[("type", text("enabled"))])),
        ("extra_body", object(&[("reasoning_effort", text("low"))])),
    ]);
    assert_eq!(
        params_with_reasoning_override(&deployment_params, &JsonMap::new()),
        deployment_params
    );
    assert_eq!(
        params_with_reasoning_override(
            &deployment_params,
            &info(&[("reasoning_effort", Json::Null)])
        ),
        deployment_params
    );
}

fn classifier_detection_reads_both_metadata_keys() {
    let origin = object(&[("internal_call_origin", text("autorouter_classifier"))]);
    assert!(is_classifier_internal_call(&info(&[(
        "metadata",
        origin.clone()
    )])));
    assert!(is_classifier_internal_call(&info(&[(
        "litellm_metadata",
        origin
    )])));
    assert!(!is_classifier_internal_call(&info(&[(
        "metadata",
        object(&[("internal_call_origin", text("other"))])
    )])));
    assert!(!is_classifier_internal_call(&info(&[(
        "metadata",
        text("x")
    )])));
    assert!(!is_classifier_internal_call(&JsonMap::new()));
}

fn a_classifier_effort_the_target_refuses_is_dropped() {
    let deployment = deployment("m", "m1");
    let mut kwargs = info(&[
        ("reasoning_effort", text("high")),
        (
            "metadata",
            object(&[("internal_call_origin", text("autorouter_classifier"))]),
        ),
        (
            "proxy_server_request",
            object(&[(
                "body",
                object(&[("reasoning_effort", text("high")), ("model", text("m"))]),
            )]),
        ),
    ]);
    let warning = drop_classifier_effort(&deployment, "m", &mut kwargs, &Supports(&["seed"]));
    assert_eq!(
        warning.as_deref(),
        Some(
            "litellm.router.py: dropping classifier reasoning_effort for model=m because the selected deployment does not support it"
        )
    );
    assert_eq!(kwargs.get("reasoning_effort"), None);
    assert_eq!(
        kwargs.get("proxy_server_request"),
        Some(&object(&[("body", object(&[("model", text("m"))]))]))
    );
}

fn a_classifier_effort_the_target_declares_stays() {
    let mut deployment = deployment("m", "m1");
    deployment.litellm_params.insert(
        "allowed_openai_params".to_owned(),
        Json::List(vec![text("reasoning_effort")]),
    );
    let mut kwargs = info(&[
        ("reasoning_effort", text("high")),
        (
            "metadata",
            object(&[("internal_call_origin", text("autorouter_classifier"))]),
        ),
    ]);
    assert_eq!(
        drop_classifier_effort(&deployment, "m", &mut kwargs, &Supports(&["seed"])),
        None
    );
    assert_eq!(kwargs.get("reasoning_effort"), Some(&text("high")));
}

fn a_request_that_is_not_a_classifier_call_keeps_its_effort() {
    let deployment = deployment("m", "m1");
    let mut kwargs = info(&[
        ("reasoning_effort", text("high")),
        (
            "metadata",
            object(&[("internal_call_origin", text("other"))]),
        ),
    ]);
    assert_eq!(
        drop_classifier_effort(&deployment, "m", &mut kwargs, &Supports(&["seed"])),
        None
    );
    assert_eq!(kwargs.get("reasoning_effort"), Some(&text("high")));
}

fn tier_params_drop_what_no_deployment_takes() {
    let tier = info(&[("temperature", number(1)), ("max_tokens", number(5))]);
    let deployments = vec![deployment("m", "m1")];
    let accepted = tier_params_the_target_accepts(
        "m",
        &tier,
        &JsonMap::new(),
        &deployments,
        &Supports(&["seed"]),
    );
    assert_eq!(accepted.params, info(&[("max_tokens", number(5))]));
    assert_eq!(
        accepted.dropped,
        BTreeSet::from([String::from("temperature")])
    );
}

fn a_param_one_deployment_takes_survives() {
    let tier = info(&[("temperature", number(1))]);
    let deployments = vec![deployment("m", "m1"), deployment("m", "m2")];
    let accepted = tier_params_the_target_accepts(
        "m",
        &tier,
        &JsonMap::new(),
        &deployments,
        &Supports(&["temperature"]),
    );
    assert_eq!(accepted.params, tier);
    assert!(accepted.dropped.is_empty());
}

fn an_allowlisted_tier_param_survives() {
    let tier = info(&[
        ("temperature", number(1)),
        (
            "allowed_openai_params",
            Json::List(vec![text("temperature")]),
        ),
    ]);
    let deployments = vec![deployment("m", "m1")];
    let accepted = tier_params_the_target_accepts(
        "m",
        &tier,
        &JsonMap::new(),
        &deployments,
        &Supports(&["seed"]),
    );
    assert_eq!(accepted.params, tier);
}

fn a_request_allowlist_and_an_unresolvable_provider_keep_the_param() {
    let tier = info(&[("temperature", number(1))]);
    let request = info(&[(
        "allowed_openai_params",
        Json::List(vec![text("temperature")]),
    )]);
    let deployments = vec![deployment("m", "m1")];
    assert_eq!(
        tier_params_the_target_accepts("m", &tier, &request, &deployments, &Supports(&["seed"]))
            .params,
        tier
    );
    assert_eq!(
        tier_params_the_target_accepts("m", &tier, &JsonMap::new(), &deployments, &Unresolved)
            .params,
        tier
    );
}

fn tier_params_without_deployments_are_untouched() {
    let tier = info(&[("temperature", number(1))]);
    let accepted =
        tier_params_the_target_accepts("m", &tier, &JsonMap::new(), &[], &Supports(&["seed"]));
    assert_eq!(accepted.params, tier);
    assert!(accepted.dropped.is_empty());
}

async fn the_stage_narrows_to_the_group_members() {
    let facts = RequestFacts::default();
    let request = route_request("tiers", &facts);
    let services = GroupServices {
        name: String::from("tiers"),
        group: priority_group("tiers", &[("a", 2)]),
    };
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("a", "a1"), deployment("b", "b1")],
    };
    let narrowed = context::apply(RouteStage::RoutingGroups, set, &route)
        .await
        .unwrap();
    assert_eq!(narrowed.deployments.len(), 1);
    assert_eq!(narrowed.deployments[0].id(), Some("a1"));
    assert_eq!(
        narrowed.deployments[0].litellm_params.get("order"),
        Some(&number(2))
    );
}

async fn a_strategy_group_keeps_its_members_unordered() {
    let facts = RequestFacts::default();
    let request = route_request("cheap", &facts);
    let services = GroupServices {
        name: String::from("cheap"),
        group: group("cheap", &["a"], "simple-shuffle"),
    };
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("a", "a1"), deployment("b", "b1")],
    };
    let narrowed = context::apply(RouteStage::RoutingGroups, set, &route)
        .await
        .unwrap();
    assert_eq!(narrowed.deployments.len(), 1);
    assert_eq!(narrowed.deployments[0].id(), Some("a1"));
    assert_eq!(narrowed.deployments[0].litellm_params.get("order"), None);
}

async fn the_stage_passes_a_pool_without_a_group_through() {
    let facts = RequestFacts::default();
    let request = route_request("plain", &facts);
    let services = GroupServices {
        name: String::from("tiers"),
        group: group("tiers", &["a"], "simple-shuffle"),
    };
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("a", "a1"), deployment("b", "b1")],
    };
    assert_eq!(
        context::apply(RouteStage::RoutingGroups, set.clone(), &route)
            .await
            .unwrap(),
        set
    );
}

async fn the_stage_leaves_a_specific_deployment_call_alone() {
    let facts = RequestFacts::default();
    let mut request = route_request("tiers", &facts);
    request.specific_deployment = true;
    let services = GroupServices {
        name: String::from("tiers"),
        group: group("tiers", &["a"], "simple-shuffle"),
    };
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("b", "b1")],
    };
    assert_eq!(
        context::apply(RouteStage::RoutingGroups, set.clone(), &route)
            .await
            .unwrap(),
        set
    );
}

#[rstest]
#[case::parse_reads_a_strategy_group(settled(parse_reads_a_strategy_group))]
#[case::parse_reads_a_priority_group(settled(parse_reads_a_priority_group))]
#[case::unknown_members_warn_once_each(settled(unknown_members_warn_once_each))]
#[case::without_known_models_nothing_warns(settled(without_known_models_nothing_warns))]
#[case::no_groups_parse_to_nothing(settled(no_groups_parse_to_nothing))]
#[case::priority_order_is_written_on_the_member(settled(priority_order_is_written_on_the_member))]
#[case::other_groups_keep_their_deployments(settled(other_groups_keep_their_deployments))]
#[case::rows_rename_and_strip_access_groups(settled(rows_rename_and_strip_access_groups))]
#[case::rows_cover_every_callable_group(settled(rows_cover_every_callable_group))]
#[case::a_shadowed_group_is_not_callable(settled(a_shadowed_group_is_not_callable))]
#[case::model_membership_feeds_the_selector_scope(settled(
    model_membership_feeds_the_selector_scope
))]
#[case::dispatch_groups_carry_strategies_and_selectors(settled(
    dispatch_groups_carry_strategies_and_selectors
))]
#[case::alternatives_count_member_deployments(settled(alternatives_count_member_deployments))]
#[case::declared_levels_win_over_flags(settled(declared_levels_win_over_flags))]
#[case::flags_advertise_the_polarity_of_each_level(settled(
    flags_advertise_the_polarity_of_each_level
))]
#[case::a_mapped_model_without_flags_is_known_empty(settled(
    a_mapped_model_without_flags_is_known_empty
))]
#[case::explicit_false_empties_even_with_levels(settled(explicit_false_empties_even_with_levels))]
#[case::the_bare_twin_supplies_the_flags(settled(the_bare_twin_supplies_the_flags))]
#[case::the_own_flag_wins_over_the_bare_twin(settled(the_own_flag_wins_over_the_bare_twin))]
#[case::declared_levels_read_the_prefixed_entry_then_the_bare_one(settled(
    declared_levels_read_the_prefixed_entry_then_the_bare_one
))]
#[case::intersection_keeps_the_advertisement_order(settled(
    intersection_keeps_the_advertisement_order
))]
#[case::an_unknown_side_never_narrows(settled(an_unknown_side_never_narrows))]
#[case::the_group_aggregate_intersects_known_deployments(settled(
    the_group_aggregate_intersects_known_deployments
))]
#[case::the_group_aggregate_clears_on_an_unknown_deployment(settled(
    the_group_aggregate_clears_on_an_unknown_deployment
))]
#[case::catalog_mapping_needs_both_modes(settled(catalog_mapping_needs_both_modes))]
#[case::nearest_rounds_up_to_the_weakest_declared_level(settled(
    nearest_rounds_up_to_the_weakest_declared_level
))]
#[case::nearest_rounds_down_above_every_level(settled(nearest_rounds_down_above_every_level))]
#[case::nearest_leaves_a_declared_level_and_none_alone(settled(
    nearest_leaves_a_declared_level_and_none_alone
))]
#[case::the_none_flag_follows_the_provider_family(settled(
    the_none_flag_follows_the_provider_family
))]
#[case::effort_wire_names_round_trip(settled(effort_wire_names_round_trip))]
#[case::a_request_effort_drops_every_native_carrier(settled(
    a_request_effort_drops_every_native_carrier
))]
#[case::without_a_request_effort_every_carrier_stays(settled(
    without_a_request_effort_every_carrier_stays
))]
#[case::classifier_detection_reads_both_metadata_keys(settled(
    classifier_detection_reads_both_metadata_keys
))]
#[case::a_classifier_effort_the_target_refuses_is_dropped(settled(
    a_classifier_effort_the_target_refuses_is_dropped
))]
#[case::a_classifier_effort_the_target_declares_stays(settled(
    a_classifier_effort_the_target_declares_stays
))]
#[case::a_request_that_is_not_a_classifier_call_keeps_its_effort(settled(
    a_request_that_is_not_a_classifier_call_keeps_its_effort
))]
#[case::tier_params_drop_what_no_deployment_takes(settled(
    tier_params_drop_what_no_deployment_takes
))]
#[case::a_param_one_deployment_takes_survives(settled(a_param_one_deployment_takes_survives))]
#[case::an_allowlisted_tier_param_survives(settled(an_allowlisted_tier_param_survives))]
#[case::a_request_allowlist_and_an_unresolvable_provider_keep_the_param(settled(
    a_request_allowlist_and_an_unresolvable_provider_keep_the_param
))]
#[case::tier_params_without_deployments_are_untouched(settled(
    tier_params_without_deployments_are_untouched
))]
#[case::the_stage_narrows_to_the_group_members(the_stage_narrows_to_the_group_members())]
#[case::a_strategy_group_keeps_its_members_unordered(a_strategy_group_keeps_its_members_unordered())]
#[case::the_stage_passes_a_pool_without_a_group_through(
    the_stage_passes_a_pool_without_a_group_through()
)]
#[case::the_stage_leaves_a_specific_deployment_call_alone(
    the_stage_leaves_a_specific_deployment_call_alone()
)]
#[tokio::test]
async fn routing_groups_behaviors<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    case.await;
}

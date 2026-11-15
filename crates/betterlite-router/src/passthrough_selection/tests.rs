#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::error::PassthroughSelectionError;
use crate::deployment_catalog::error::CatalogLookupError;
use crate::static_eligibility::error::EligibilityError;
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroupRoute, RoutingGroups};
use crate::strategy_dispatch::selection::SelectorCall;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{ResolutionSource, StrategyOverride};

use self::support::{
    LITELLM_MODEL, MODEL, Metric, Picker, Recorded, aliased, ask, assert_invalid_field,
    assert_not_configured, assert_rate_limited, assert_selected, facts, flag, flagged, gpt,
    json_mode, metric_setting, number, shuffle, signed, spec, supported_in, text, weighted,
    with_param, world_with,
};

mod support;

// The id arm returns the one deployment it names without a draw.
fn deployment_id() {
    let mut world = shuffle(vec![weighted("chosen", 1)]);
    let mut picker = Picker::default();
    let selected = world.call(&mut picker, &ask("chosen", &facts())).unwrap();
    assert_selected(&selected, MODEL, "chosen");
    assert_eq!(selected.resolution.strategy, RoutingStrategy::SimpleShuffle);
    assert_eq!(selected.resolution.source, ResolutionSource::Default);
    assert_eq!(selected.override_pre_call, None);
    assert!(picker.calls.is_empty());
}

fn only_flagged() {
    let mut world = shuffle(vec![weighted("only", 1)]);
    let selected = world
        .call(&mut Picker::default(), &ask(MODEL, &facts()))
        .unwrap();
    assert_selected(&selected, MODEL, "only");
}

fn unflagged_leaves() {
    let mut world = shuffle(vec![
        with_param(gpt("skipped"), "weight", number(5)),
        weighted("kept", 1),
    ]);
    let selected = world
        .call(&mut Picker::default(), &ask(MODEL, &facts()))
        .unwrap();
    assert_selected(&selected, MODEL, "kept");
}

fn metric_decides(metric: Metric) {
    let mut world = shuffle(vec![
        metric_setting(metric, "first", 0),
        metric_setting(metric, "second", 5),
    ]);
    let selected = world
        .call(&mut Picker::default(), &ask(MODEL, &facts()))
        .unwrap();
    assert_selected(&selected, MODEL, "second");
}

// The metric on the first deployment is what the draw reads: without it the draw is uniform and lands on the second.
fn weight_decides_first() {
    let mut world = shuffle(vec![
        metric_setting(Metric::Weight, "first", 5),
        metric_setting(Metric::Weight, "second", 0),
    ]);
    let selected = world
        .call(&mut Picker::default(), &ask(MODEL, &facts()))
        .unwrap();
    assert_selected(&selected, MODEL, "first");
}

fn provider_filter() {
    let mut world = shuffle(vec![
        with_param(
            flagged(spec(MODEL, "modern", "openai/gpt-4o")),
            "weight",
            number(1),
        ),
        with_param(
            flagged(spec(MODEL, "legacy", "openai/gpt-4")),
            "weight",
            number(1),
        ),
    ]);
    world.enable_pre_call_checks = true;
    let selected = world
        .call(&mut Picker::default(), &ask(MODEL, &json_mode()))
        .unwrap();
    assert_selected(&selected, MODEL, "modern");
}

fn specific_deployment_pool() {
    let mut world = shuffle(vec![
        with_param(gpt("alpha"), "weight", number(1)),
        weighted("beta", 1),
    ]);
    let selected = world
        .call(
            &mut Picker::default(),
            &ask(LITELLM_MODEL, &facts()).with_specific(),
        )
        .unwrap();
    assert_selected(&selected, MODEL, "beta");
}

fn override_selector() {
    let mut world = shuffle(vec![weighted("a", 1), weighted("b", 1)]);
    let mut picker = Picker {
        result: Some(1),
        calls: Vec::new(),
    };
    let selected = world
        .call(
            &mut picker,
            &ask(MODEL, &facts()).with_strategy(StrategyOverride::Text("least-busy")),
        )
        .unwrap();
    assert_selected(&selected, MODEL, "b");
    assert_eq!(selected.resolution.strategy, RoutingStrategy::LeastBusy);
    assert_eq!(
        selected.resolution.source,
        ResolutionSource::RequestOverride
    );
    let selector = selected.resolution.selector.unwrap();
    assert_eq!(selected.override_pre_call, Some(selector));
    assert_eq!(
        picker.calls,
        vec![Recorded {
            selector,
            call: SelectorCall::LeastBusy,
            available: 2,
        }]
    );
}

fn default_selector() {
    let mut world = world_with(
        RoutingStrategy::LeastBusy,
        vec![weighted("a", 1), weighted("b", 1)],
    );
    let mut picker = Picker {
        result: Some(0),
        calls: Vec::new(),
    };
    let selected = world.call(&mut picker, &ask(MODEL, &facts())).unwrap();
    assert_selected(&selected, MODEL, "a");
    assert_eq!(selected.resolution.strategy, RoutingStrategy::LeastBusy);
    assert_eq!(selected.resolution.source, ResolutionSource::Default);
    // the router's own selector is not an override selector, so the pass-through call runs no pre-call check
    assert_eq!(selected.override_pre_call, None);
    assert_eq!(picker.calls.len(), 1);
}

fn ignored_override() {
    let mut world = shuffle(vec![weighted("only", 1)]);
    let mut picker = Picker::default();
    let selected = world
        .call(
            &mut picker,
            &ask(MODEL, &facts()).with_strategy(StrategyOverride::Text("lar1")),
        )
        .unwrap();
    assert_selected(&selected, MODEL, "only");
    assert_eq!(selected.resolution.strategy, RoutingStrategy::SimpleShuffle);
    assert_eq!(selected.override_pre_call, None);
    assert!(picker.calls.is_empty());
    assert_eq!(
        selected
            .resolution
            .ignored_override
            .and_then(|ignored| ignored.raw),
        Some("lar1".to_owned())
    );
}

fn alias_routing_model() {
    let mut world = shuffle(vec![with_param(
        flagged(spec("target", "only", LITELLM_MODEL)),
        "weight",
        number(1),
    )]);
    world
        .model_group_alias
        .insert("alias".to_owned(), "target".to_owned());
    world.groups = RoutingGroups::new(vec![RoutingGroupRoute {
        name: "shuffle-group".to_owned(),
        models: vec!["target".to_owned()],
        strategy: GroupStrategy::Routing(RoutingStrategy::SimpleShuffle),
        selector: None,
    }])
    .unwrap();
    let selected = world
        .call(&mut Picker::default(), &ask("alias", &facts()))
        .unwrap();
    assert_selected(&selected, "target", "only");
    assert_eq!(
        selected.resolution.source,
        ResolutionSource::RoutingGroup("shuffle-group".to_owned())
    );
}

#[rstest]
#[case::deployment_id(Behavior::DeploymentId)]
#[case::only_flagged(Behavior::OnlyFlagged)]
#[case::unflagged_leaves(Behavior::UnflaggedLeaves)]
#[case::weight_decides(Behavior::WeightDecides)]
#[case::weight_decides_first(Behavior::WeightDecidesFirst)]
#[case::rpm_decides(Behavior::RpmDecides)]
#[case::tpm_decides(Behavior::TpmDecides)]
#[case::provider_filter(Behavior::ProviderFilter)]
#[case::specific_deployment(Behavior::SpecificDeployment)]
#[case::override_selector(Behavior::OverrideSelector)]
#[case::default_selector(Behavior::DefaultSelector)]
#[case::ignored_override(Behavior::IgnoredOverride)]
#[case::alias_routing_model(Behavior::AliasRoutingModel)]
fn passthrough_selection_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::DeploymentId => deployment_id(),
        Behavior::OnlyFlagged => only_flagged(),
        Behavior::UnflaggedLeaves => unflagged_leaves(),
        Behavior::WeightDecides => metric_decides(Metric::Weight),
        Behavior::WeightDecidesFirst => weight_decides_first(),
        Behavior::RpmDecides => metric_decides(Metric::Rpm),
        Behavior::TpmDecides => metric_decides(Metric::Tpm),
        Behavior::ProviderFilter => provider_filter(),
        Behavior::SpecificDeployment => specific_deployment_pool(),
        Behavior::OverrideSelector => override_selector(),
        Behavior::DefaultSelector => default_selector(),
        Behavior::IgnoredOverride => ignored_override(),
        Behavior::AliasRoutingModel => alias_routing_model(),
    }
}

#[derive(Clone, Copy, Debug)]
enum Behavior {
    DeploymentId,
    OnlyFlagged,
    UnflaggedLeaves,
    WeightDecides,
    WeightDecidesFirst,
    RpmDecides,
    TpmDecides,
    ProviderFilter,
    SpecificDeployment,
    OverrideSelector,
    DefaultSelector,
    IgnoredOverride,
    AliasRoutingModel,
}

fn flag_value(value: Option<Json>) {
    let deployment = match value {
        Some(setting) => with_param(gpt("only"), "use_in_pass_through", setting),
        None => gpt("only"),
    };
    let mut world = shuffle(vec![deployment]);
    assert_not_configured(
        world.call(&mut Picker::default(), &ask(MODEL, &facts())),
        MODEL,
    );
}

fn flag_wrong_type() {
    let mut world = shuffle(vec![with_param(
        gpt("only"),
        "use_in_pass_through",
        text("true"),
    )]);
    assert_invalid_field(
        world.call(&mut Picker::default(), &ask(MODEL, &facts())),
        "only",
        "use_in_pass_through",
        "a boolean",
    );
}

fn invalid_weight(value: Json) {
    let mut world = shuffle(vec![with_param(weighted("only", 1), "weight", value)]);
    assert_invalid_field(
        world.call(&mut Picker::default(), &ask(MODEL, &facts())),
        "only",
        "weight",
        "a non-negative whole number below 2^32",
    );
}

fn no_flagged() {
    let mut world = shuffle(vec![gpt("plain")]);
    assert_not_configured(
        world.call(&mut Picker::default(), &ask(MODEL, &facts())),
        MODEL,
    );
}

fn weight_at_the_limit() {
    let mut world = shuffle(vec![weighted("only", u64::from(u32::MAX))]);
    let selected = world
        .call(&mut Picker::default(), &ask(MODEL, &facts()))
        .unwrap();
    assert_selected(&selected, MODEL, "only");
}

fn denied_deployment_id() {
    let mut world = shuffle(vec![gpt("plain")]);
    let result = world.call(&mut Picker::default(), &ask("plain", &facts()));
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::DeploymentNotConfigured { deployment_id } = &error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(deployment_id, "plain");
    assert_eq!(
        error.to_string(),
        "Deployment plain does not support pass-through endpoint (use_in_pass_through=False)"
    );
}

fn blocked_deployment_id() {
    let mut deployment = flagged(gpt("paused"));
    deployment
        .model_info
        .insert("blocked".to_owned(), flag(true));
    let mut world = shuffle(vec![deployment]);
    let result = world.call(&mut Picker::default(), &ask("paused", &facts()));
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::Eligibility(EligibilityError::Blocked { model }) = &error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(model, LITELLM_MODEL);
    assert_eq!(
        error.to_string(),
        format!("Model '{LITELLM_MODEL}' is currently paused and cannot accept requests.")
    );
}

fn empty_pool() {
    let mut world = shuffle(vec![
        with_param(
            flagged(spec(MODEL, "legacy", "openai/gpt-4")),
            "weight",
            number(1),
        ),
        with_param(
            flagged(spec(MODEL, "old", "openai/gpt-3.5-turbo-16k")),
            "weight",
            number(1),
        ),
    ]);
    world.enable_pre_call_checks = true;
    assert_rate_limited(
        world.call(&mut Picker::default(), &ask(MODEL, &json_mode())),
        MODEL,
    );
}

fn environment_empties_the_group() {
    let mut world = shuffle(vec![supported_in(weighted("only", 1), &["development"])]);
    let result = world.call(&mut Picker::default(), &ask(MODEL, &facts()));
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::Catalog(CatalogLookupError::NoHealthyDeployments { model }) =
        &error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(model, MODEL);
}

fn environment_and_pre_call_leave_nothing() {
    let legacy = supported_in(
        with_param(
            flagged(spec(MODEL, "legacy", "openai/gpt-4")),
            "weight",
            number(1),
        ),
        &["development"],
    );
    let old = with_param(
        flagged(spec(MODEL, "old", "openai/gpt-3.5-turbo-16k")),
        "weight",
        number(1),
    );
    let mut world = shuffle(vec![legacy, old]);
    world.enable_pre_call_checks = true;
    assert_rate_limited(
        world.call(&mut Picker::default(), &ask(MODEL, &json_mode())),
        MODEL,
    );
}

fn not_configured_names_the_requested_model() {
    let mut world = aliased(vec![gpt("plain")], RoutingStrategy::SimpleShuffle);
    assert_not_configured(
        world.call(&mut Picker::default(), &ask("alias", &facts())),
        "alias",
    );
}

fn empty_pool_names_the_mapped_model() {
    let mut world = aliased(
        vec![supported_in(weighted("only", 1), &["development"])],
        RoutingStrategy::SimpleShuffle,
    );
    let result = world.call(&mut Picker::default(), &ask("alias", &facts()));
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::Catalog(CatalogLookupError::NoHealthyDeployments { model }) =
        &error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(model, MODEL);
}

fn selector_none() {
    let mut world = world_with(
        RoutingStrategy::LeastBusy,
        vec![weighted("a", 1), weighted("b", 1)],
    );
    let mut picker = Picker {
        result: None,
        calls: Vec::new(),
    };
    assert_rate_limited(world.call(&mut picker, &ask(MODEL, &facts())), MODEL);
    assert_eq!(picker.calls.len(), 1);
    assert_eq!(picker.calls[0].available, 2);
}

fn selector_none_names_the_requested_model() {
    let mut world = aliased(vec![weighted("only", 1)], RoutingStrategy::LeastBusy);
    let mut picker = Picker {
        result: None,
        calls: Vec::new(),
    };
    assert_rate_limited(world.call(&mut picker, &ask("alias", &facts())), "alias");
    assert_eq!(picker.calls.len(), 1);
}

fn selector_out_of_range() {
    let mut world = world_with(
        RoutingStrategy::LeastBusy,
        vec![weighted("a", 1), weighted("b", 1)],
    );
    let mut picker = Picker {
        result: Some(2),
        calls: Vec::new(),
    };
    let result = world.call(&mut picker, &ask(MODEL, &facts()));
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::SelectorOutOfRange {
        index, available, ..
    } = &error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(*index, 2);
    assert_eq!(*available, 2);
    assert!(
        error
            .to_string()
            .ends_with("chose deployment 2, but only 2 pass-through deployments exist")
    );
}

fn selector_last_index() {
    let mut world = world_with(
        RoutingStrategy::LeastBusy,
        vec![weighted("a", 1), weighted("b", 1)],
    );
    let mut picker = Picker {
        result: Some(1),
        calls: Vec::new(),
    };
    let selected = world.call(&mut picker, &ask(MODEL, &facts())).unwrap();
    assert_selected(&selected, MODEL, "b");
}

fn lar1_has_no_selector() {
    let mut world = world_with(RoutingStrategy::Lar1, vec![weighted("only", 1)]);
    let mut picker = Picker::default();
    assert_rate_limited(world.call(&mut picker, &ask(MODEL, &facts())), MODEL);
    assert!(picker.calls.is_empty());
}

fn specific_unflagged() {
    let mut world = shuffle(vec![gpt("a")]);
    assert_not_configured(
        world.call(
            &mut Picker::default(),
            &ask(LITELLM_MODEL, &facts()).with_specific(),
        ),
        LITELLM_MODEL,
    );
}

fn alias_map_does_not_steer_routing() {
    let mut world = shuffle(vec![with_param(
        flagged(spec("public", "only", LITELLM_MODEL)),
        "weight",
        number(1),
    )]);
    world
        .model_alias_map
        .insert("public".to_owned(), "target".to_owned());
    world.groups = RoutingGroups::new(vec![RoutingGroupRoute {
        name: "cost-group".to_owned(),
        models: vec!["target".to_owned()],
        strategy: GroupStrategy::Routing(RoutingStrategy::CostBasedRouting),
        selector: None,
    }])
    .unwrap();
    let selected = world
        .call(&mut Picker::default(), &ask("public", &facts()))
        .unwrap();
    assert_selected(&selected, "public", "only");
    assert_eq!(selected.resolution.source, ResolutionSource::Default);
}

fn alias_empty_target_keeps_the_model() {
    let mut world = shuffle(vec![weighted("only", 1)]);
    world
        .model_group_alias
        .insert(MODEL.to_owned(), String::new());
    world.groups = RoutingGroups::new(vec![RoutingGroupRoute {
        name: "gpt-group".to_owned(),
        models: vec![MODEL.to_owned()],
        strategy: GroupStrategy::Routing(RoutingStrategy::SimpleShuffle),
        selector: None,
    }])
    .unwrap();
    let selected = world
        .call(&mut Picker::default(), &ask(MODEL, &facts()))
        .unwrap();
    assert_selected(&selected, MODEL, "only");
    assert_eq!(
        selected.resolution.source,
        ResolutionSource::RoutingGroup("gpt-group".to_owned())
    );
}

fn single_without_active_environment() {
    let deployment = supported_in(
        flagged(spec("group", "paused", LITELLM_MODEL)),
        &["production"],
    );
    let mut world = shuffle(vec![deployment]);
    world.environment = "development";
    let result = world.call(&mut Picker::default(), &ask("paused", &facts()));
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::Catalog(CatalogLookupError::NoHealthyDeployments { model }) =
        &error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(model, "paused");
}

#[rstest]
#[case::flag_absent(Boundary::FlagAbsent)]
#[case::flag_null(Boundary::FlagNull)]
#[case::flag_false(Boundary::FlagFalse)]
#[case::flag_wrong_type(Boundary::FlagWrongType)]
#[case::weight_text(Boundary::WeightText)]
#[case::weight_negative(Boundary::WeightNegative)]
#[case::weight_fraction(Boundary::WeightFraction)]
#[case::weight_integral_float(Boundary::WeightIntegralFloat)]
#[case::weight_too_large(Boundary::WeightTooLarge)]
#[case::weight_at_the_limit(Boundary::WeightAtTheLimit)]
#[case::no_flagged(Boundary::NoFlagged)]
#[case::not_configured_names_the_requested_model(Boundary::NotConfiguredNamesTheRequestedModel)]
#[case::denied_deployment_id(Boundary::DeniedDeploymentId)]
#[case::blocked_deployment_id(Boundary::BlockedDeploymentId)]
#[case::empty_pool(Boundary::EmptyPool)]
#[case::environment_empties_the_group(Boundary::EnvironmentEmptiesTheGroup)]
#[case::environment_and_pre_call_leave_nothing(Boundary::EnvironmentAndPreCallLeaveNothing)]
#[case::empty_pool_names_the_mapped_model(Boundary::EmptyPoolNamesTheMappedModel)]
#[case::selector_none(Boundary::SelectorNone)]
#[case::selector_none_names_the_requested_model(Boundary::SelectorNoneNamesTheRequestedModel)]
#[case::selector_out_of_range(Boundary::SelectorOutOfRange)]
#[case::selector_last_index(Boundary::SelectorLastIndex)]
#[case::lar1_has_no_selector(Boundary::Lar1HasNoSelector)]
#[case::specific_unflagged(Boundary::SpecificUnflagged)]
#[case::alias_map_does_not_steer_routing(Boundary::AliasMapDoesNotSteerRouting)]
#[case::alias_empty_target(Boundary::AliasEmptyTarget)]
#[case::single_without_active_environment(Boundary::SingleWithoutActiveEnvironment)]
fn passthrough_selection_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::FlagAbsent => flag_value(None),
        Boundary::FlagNull => flag_value(Some(Json::Null)),
        Boundary::FlagFalse => flag_value(Some(flag(false))),
        Boundary::FlagWrongType => flag_wrong_type(),
        Boundary::WeightText => invalid_weight(text("5")),
        Boundary::WeightNegative => invalid_weight(signed(-1)),
        Boundary::WeightFraction => {
            invalid_weight(serde_json::from_str::<Json>("1.5").unwrap_or(Json::Null));
        }
        Boundary::WeightIntegralFloat => {
            invalid_weight(serde_json::from_str::<Json>("5.0").unwrap_or(Json::Null));
        }
        Boundary::WeightTooLarge => invalid_weight(number(u64::from(u32::MAX) + 1)),
        Boundary::WeightAtTheLimit => weight_at_the_limit(),
        Boundary::NoFlagged => no_flagged(),
        Boundary::NotConfiguredNamesTheRequestedModel => not_configured_names_the_requested_model(),
        Boundary::DeniedDeploymentId => denied_deployment_id(),
        Boundary::BlockedDeploymentId => blocked_deployment_id(),
        Boundary::EmptyPool => empty_pool(),
        Boundary::EnvironmentEmptiesTheGroup => environment_empties_the_group(),
        Boundary::EnvironmentAndPreCallLeaveNothing => environment_and_pre_call_leave_nothing(),
        Boundary::EmptyPoolNamesTheMappedModel => empty_pool_names_the_mapped_model(),
        Boundary::SelectorNone => selector_none(),
        Boundary::SelectorNoneNamesTheRequestedModel => selector_none_names_the_requested_model(),
        Boundary::SelectorOutOfRange => selector_out_of_range(),
        Boundary::SelectorLastIndex => selector_last_index(),
        Boundary::Lar1HasNoSelector => lar1_has_no_selector(),
        Boundary::SpecificUnflagged => specific_unflagged(),
        Boundary::AliasMapDoesNotSteerRouting => alias_map_does_not_steer_routing(),
        Boundary::AliasEmptyTarget => alias_empty_target_keeps_the_model(),
        Boundary::SingleWithoutActiveEnvironment => single_without_active_environment(),
    }
}

#[derive(Clone, Copy, Debug)]
enum Boundary {
    FlagAbsent,
    FlagNull,
    FlagFalse,
    FlagWrongType,
    WeightText,
    WeightNegative,
    WeightFraction,
    WeightIntegralFloat,
    WeightTooLarge,
    WeightAtTheLimit,
    NoFlagged,
    NotConfiguredNamesTheRequestedModel,
    DeniedDeploymentId,
    BlockedDeploymentId,
    EmptyPool,
    EnvironmentEmptiesTheGroup,
    EnvironmentAndPreCallLeaveNothing,
    EmptyPoolNamesTheMappedModel,
    SelectorNoneNamesTheRequestedModel,
    SelectorNone,
    SelectorOutOfRange,
    SelectorLastIndex,
    Lar1HasNoSelector,
    SpecificUnflagged,
    AliasMapDoesNotSteerRouting,
    AliasEmptyTarget,
    SingleWithoutActiveEnvironment,
}

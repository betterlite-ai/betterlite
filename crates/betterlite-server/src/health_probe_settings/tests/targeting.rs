use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::health_probe_settings::targeting::{
    deployments_targeted_by_name, filter_deployments_by_id, filter_to_model_groups,
    health_check_eligible, narrow_to_target,
};
use crate::health_probe_settings::tests::support::{spec, with_team};

fn ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .filter_map(DeploymentSpec::id)
        .map(str::to_owned)
        .collect()
}

fn groups(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn unset_model_groups_keep_every_deployment() {
    let list = vec![
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
        spec("claude", Some("b"), "anthropic/claude"),
    ];
    assert_eq!(ids(&filter_to_model_groups(&list, None)), ["a", "b"]);
}

#[test]
fn model_groups_narrow_the_list() {
    let list = vec![
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
        spec("claude", Some("b"), "anthropic/claude"),
    ];
    let narrowed = filter_to_model_groups(&list, Some(&groups(&["claude"])));
    assert_eq!(ids(&narrowed), ["b"]);
}

#[test]
fn an_empty_model_group_set_keeps_nothing() {
    let list = vec![spec("gpt-4o", Some("a"), "openai/gpt-4o")];
    assert!(filter_to_model_groups(&list, Some(&groups(&[]))).is_empty());
}

#[test]
fn deduplication_drops_deployments_without_an_id() {
    let list = vec![
        spec("gpt-4o", None, "openai/gpt-4o"),
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
    ];
    assert_eq!(ids(&filter_deployments_by_id(&list)), ["a"]);
}

#[test]
fn an_empty_id_counts_as_missing() {
    let list = vec![spec("gpt-4o", Some(""), "openai/gpt-4o")];
    assert!(filter_deployments_by_id(&list).is_empty());
}

#[test]
fn deduplication_keeps_the_first_occurrence() {
    let list = vec![
        spec("gpt-4o", Some("a"), "openai/gpt-4"),
        spec("gpt-4o", Some("b"), "openai/gpt-4o"),
        spec("gpt-4o", Some("a"), "openai/gpt-4o-mini"),
    ];
    let kept = filter_deployments_by_id(&list);
    assert_eq!(ids(&kept), ["a", "b"]);
    assert_eq!(
        kept.first()
            .and_then(|spec| spec.litellm_params.get("model")),
        Some(&Json::Text("openai/gpt-4".to_owned()))
    );
}

fn with_disabled_flag(mut deployment: DeploymentSpec, disabled: Json) -> DeploymentSpec {
    deployment
        .model_info
        .insert("disable_background_health_check".to_owned(), disabled);
    deployment
}

#[test]
fn a_disabled_background_deployment_is_skipped_when_asked() {
    let list = vec![
        with_disabled_flag(spec("gpt-4o", Some("a"), "openai/gpt-4o"), Json::Bool(true)),
        spec("gpt-4o", Some("b"), "openai/gpt-4o"),
    ];
    assert_eq!(ids(&health_check_eligible(&list, true)), ["b"]);
}

#[test]
fn a_disabled_background_deployment_stays_by_default() {
    let list = vec![with_disabled_flag(
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
        Json::Bool(true),
    )];
    assert_eq!(ids(&health_check_eligible(&list, false)), ["a"]);
}

#[test]
fn only_a_boolean_true_flag_disables_a_deployment() {
    let list = vec![with_disabled_flag(
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
        Json::Text("true".to_owned()),
    )];
    assert_eq!(ids(&health_check_eligible(&list, true)), ["a"]);
}

#[test]
fn a_teams_own_public_copy_wins() {
    let list = vec![
        with_team(
            spec("internal", Some("own"), "openai/gpt-4o"),
            "team-a",
            "shared",
        ),
        spec("shared", Some("plain"), "openai/gpt-4o"),
    ];
    assert_eq!(
        ids(&deployments_targeted_by_name(
            &list,
            "shared",
            Some("team-a")
        )),
        ["own"]
    );
}

#[test]
fn deployments_named_the_model_come_after_own_copies() {
    let list = vec![
        with_team(
            spec("internal", Some("own"), "openai/gpt-4o"),
            "team-a",
            "other-name",
        ),
        spec("shared", Some("plain"), "openai/gpt-4o"),
    ];
    assert_eq!(
        ids(&deployments_targeted_by_name(
            &list,
            "shared",
            Some("team-a")
        )),
        ["plain"]
    );
}

#[test]
fn another_teams_public_name_is_not_reachable_by_a_team() {
    let list = vec![with_team(
        spec("internal", Some("own"), "openai/gpt-4o"),
        "team-a",
        "shared",
    )];
    assert!(deployments_targeted_by_name(&list, "shared", Some("team-b")).is_empty());
}

#[test]
fn a_caller_without_a_team_reaches_a_public_name() {
    let list = vec![with_team(
        spec("internal", Some("own"), "openai/gpt-4o"),
        "team-a",
        "shared",
    )];
    assert_eq!(
        ids(&deployments_targeted_by_name(&list, "shared", None)),
        ["own"]
    );
}

#[test]
fn the_deployment_model_string_is_the_last_resort() {
    let list = vec![spec("internal", Some("a"), "openai/gpt-4o")];
    assert_eq!(
        ids(&deployments_targeted_by_name(&list, "openai/gpt-4o", None)),
        ["a"]
    );
}

#[test]
fn an_unknown_name_targets_nothing() {
    let list = vec![spec("internal", Some("a"), "openai/gpt-4o")];
    assert!(deployments_targeted_by_name(&list, "mystery", None).is_empty());
}

#[test]
fn an_id_selects_one_deployment() {
    let list = vec![
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
        spec("gpt-4o", Some("b"), "openai/gpt-4o"),
    ];
    assert_eq!(ids(&narrow_to_target(&list, None, Some("b"), None)), ["b"]);
}

#[test]
fn an_unknown_id_keeps_the_whole_list() {
    let list = vec![spec("gpt-4o", Some("a"), "openai/gpt-4o")];
    assert_eq!(
        ids(&narrow_to_target(&list, None, Some("missing"), None)),
        ["a"]
    );
}

#[test]
fn an_id_wins_over_a_model_name() {
    let list = vec![
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
        spec("claude", Some("b"), "anthropic/claude"),
    ];
    assert_eq!(
        ids(&narrow_to_target(&list, Some("claude"), Some("a"), None)),
        ["a"]
    );
}

#[test]
fn no_target_keeps_the_whole_list() {
    let list = vec![
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
        spec("claude", Some("b"), "anthropic/claude"),
    ];
    assert_eq!(ids(&narrow_to_target(&list, None, None, None)), ["a", "b"]);
}

#[test]
fn a_model_narrows_by_routing() {
    let list = vec![
        spec("gpt-4o", Some("a"), "openai/gpt-4o"),
        spec("claude", Some("b"), "anthropic/claude"),
    ];
    assert_eq!(
        ids(&narrow_to_target(&list, Some("claude"), None, None)),
        ["b"]
    );
}

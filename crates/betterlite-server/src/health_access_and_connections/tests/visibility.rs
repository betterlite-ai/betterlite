use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use rstest::rstest;

use crate::health_access_and_connections::visibility::{
    HealthCaller, LookupFailure, caller_may_probe_deployment, health_accessible_model_names,
    is_proxy_admin, resolve_targeted_model_ids,
};

use super::support::{deployment, failing_router, fake_router, object, team_deployment, text};

fn caller(
    role: Option<&str>,
    models: &[&str],
    team_models: &[&str],
    team_id: Option<&str>,
) -> HealthCaller {
    HealthCaller {
        user_id: Some("caller-1".to_owned()),
        user_role: role.map(str::to_owned),
        team_id: team_id.map(str::to_owned),
        models: models.iter().map(|model| (*model).to_owned()).collect(),
        team_models: team_models
            .iter()
            .map(|model| (*model).to_owned())
            .collect(),
    }
}

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[rstest]
#[case::proxy_admin("proxy_admin")]
#[case::proxy_admin_viewer("proxy_admin_viewer")]
fn the_admin_roles_are_admin(#[case] role: &str) {
    assert!(is_proxy_admin(&caller(Some(role), &[], &[], None)));
}

#[rstest]
#[case::internal_user("internal_user")]
#[case::org_admin("org_admin")]
#[case::upper_case("PROXY_ADMIN")]
#[case::empty("")]
fn the_other_roles_are_not_admin(#[case] role: &str) {
    assert!(!is_proxy_admin(&caller(Some(role), &[], &[], None)));
}

#[test]
fn a_caller_without_a_role_is_not_an_admin() {
    assert!(!is_proxy_admin(&caller(None, &[], &[], None)));
}

#[test]
fn an_empty_grant_or_the_proxy_sentinel_scopes_nothing() {
    let failing = failing_router();
    let unrestricted = health_accessible_model_names(&caller(None, &[], &[], None), Some(&failing));
    assert_eq!(unrestricted.unwrap(), None);
    let sentinel = caller(None, &["all-proxy-models"], &[], None);
    assert_eq!(
        health_accessible_model_names(&sentinel, Some(&failing)).unwrap(),
        None
    );
}

#[test]
fn a_key_without_a_router_sees_its_resolved_names() {
    let scoped = caller(None, &["gpt-4o", "claude"], &[], None);
    assert_eq!(
        health_accessible_model_names(&scoped, None).unwrap(),
        Some(names(&["gpt-4o", "claude"]))
    );
    let team = caller(None, &["all-team-models"], &["team-model"], Some("t1"));
    assert_eq!(
        health_accessible_model_names(&team, None).unwrap(),
        Some(names(&["team-model"]))
    );
    let orphan = caller(None, &["all-team-models"], &["team-model"], None);
    assert_eq!(health_accessible_model_names(&orphan, None).unwrap(), None);
}

#[test]
fn a_router_expands_grants_through_names_and_access_groups() {
    let mut router = fake_router(vec![
        deployment("gpt-4o", "d1", Vec::new(), Vec::new()),
        team_deployment("team-model", "d2", "t1"),
    ]);
    router
        .access_groups
        .insert("beta".to_owned(), vec!["gpt-4o".to_owned()]);
    let group = caller(None, &["beta"], &[], None);
    assert_eq!(
        health_accessible_model_names(&group, Some(&router)).unwrap(),
        Some(names(&["gpt-4o"]))
    );
    let literal = caller(None, &["gpt-4o"], &[], None);
    assert_eq!(
        health_accessible_model_names(&literal, Some(&router)).unwrap(),
        Some(names(&["gpt-4o"]))
    );
    let team = caller(None, &["all-team-models"], &["team-model"], Some("t1"));
    assert_eq!(
        health_accessible_model_names(&team, Some(&router)).unwrap(),
        Some(names(&["team-model"]))
    );
}

#[test]
fn a_lookup_failure_reaches_the_caller() {
    let failing = failing_router();
    let scoped = caller(None, &["gpt-4o"], &[], None);
    assert!(matches!(
        health_accessible_model_names(&scoped, Some(&failing)),
        Err(LookupFailure::Lookup(_))
    ));
}

#[test]
fn another_teams_deployment_is_never_in_scope() {
    let other_team = team_deployment("gpt-4o", "d1", "t2");
    assert!(!caller_may_probe_deployment(
        &other_team,
        None,
        None,
        Some("t1"),
        false
    ));
    assert!(caller_may_probe_deployment(
        &other_team,
        None,
        None,
        Some("t2"),
        false
    ));
    assert!(!caller_may_probe_deployment(
        &other_team,
        None,
        None,
        None,
        false
    ));
    assert!(caller_may_probe_deployment(
        &other_team,
        None,
        None,
        None,
        true
    ));
}

#[test]
fn a_missing_allowlist_admits_every_deployment_in_scope() {
    let shared = deployment("gpt-4o", "d1", Vec::new(), Vec::new());
    assert!(caller_may_probe_deployment(
        &shared, None, None, None, false
    ));
}

#[test]
fn without_a_router_a_grant_matches_the_proxy_name() {
    let shared = deployment("gpt-4o", "d1", Vec::new(), Vec::new());
    let allowed = names(&["gpt-4o"]);
    assert!(caller_may_probe_deployment(
        &shared,
        Some(&allowed),
        None,
        None,
        false
    ));
    let elsewhere = names(&["claude"]);
    assert!(!caller_may_probe_deployment(
        &shared,
        Some(&elsewhere),
        None,
        None,
        false
    ));
}

#[test]
fn with_a_router_a_team_public_name_is_in_scope() {
    let router = fake_router(Vec::new());
    let mut spec = deployment("internal-name", "d1", Vec::new(), Vec::new());
    spec.model_info.insert("team_id".to_owned(), text("t1"));
    spec.model_info
        .insert("team_public_model_name".to_owned(), text("public-name"));
    let allowed = names(&["public-name"]);
    assert!(caller_may_probe_deployment(
        &spec,
        Some(&allowed),
        Some(&router),
        Some("t1"),
        false
    ));
    assert!(!caller_may_probe_deployment(
        &spec,
        Some(&allowed),
        Some(&router),
        None,
        false
    ));
}

#[test]
fn a_model_id_wins_and_a_model_name_reaches_the_routed_rows() {
    let list = vec![
        deployment("gpt", "d1", Vec::new(), Vec::new()),
        deployment("gpt", "d2", Vec::new(), Vec::new()),
        deployment(
            "alias",
            "d3",
            vec![("model", text("openai/alias"))],
            Vec::new(),
        ),
    ];
    assert_eq!(
        resolve_targeted_model_ids(&list, Some("gpt"), Some("d2"), None),
        Some(names(&["d2"]))
    );
    assert_eq!(
        resolve_targeted_model_ids(&list, None, Some("missing"), None),
        Some(BTreeSet::new())
    );
    assert_eq!(
        resolve_targeted_model_ids(&list, Some("gpt"), None, None),
        Some(names(&["d1", "d2"]))
    );
    assert_eq!(
        resolve_targeted_model_ids(&list, Some("gpt"), Some(""), None),
        Some(names(&["d1", "d2"]))
    );
    assert_eq!(
        resolve_targeted_model_ids(&list, Some("openai/alias"), None, None),
        Some(names(&["d3"]))
    );
    assert_eq!(resolve_targeted_model_ids(&list, None, None, None), None);
    assert_eq!(
        resolve_targeted_model_ids(&list, Some(""), None, None),
        None
    );
    let unnamed = DeploymentSpec {
        model_name: "gpt".to_owned(),
        litellm_params: JsonObject::new(),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    };
    assert_eq!(
        resolve_targeted_model_ids(&[unnamed], Some("gpt"), None, None),
        Some(BTreeSet::new())
    );
    let blank_id = DeploymentSpec {
        model_name: "gpt".to_owned(),
        litellm_params: JsonObject::new(),
        model_info: object(vec![("id", text(""))]),
        extra: JsonObject::new(),
    };
    assert_eq!(
        resolve_targeted_model_ids(&[blank_id], Some("gpt"), None, None),
        Some(BTreeSet::new())
    );
}

#[test]
fn a_null_team_owner_is_shared_with_every_caller() {
    let shared = deployment("gpt-4o", "d1", Vec::new(), vec![("team_id", Json::Null)]);
    assert!(caller_may_probe_deployment(
        &shared,
        None,
        None,
        Some("t1"),
        false
    ));
}

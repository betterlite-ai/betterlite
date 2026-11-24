use rstest::rstest;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_group_lookups::access_groups::AccessGroups;
use crate::model_group_lookups::error::AccessGroupError;
use crate::model_group_usage::access_filter::filter_by_access_groups;
use crate::model_group_usage::access_filter::{AccessFilterRequest, KeyGrants};

use super::support::{World, grants, grouped, text};

#[test]
fn access_group_filter() {
    let rows = [
        grouped("group", "a", &["alpha", "beta"]),
        grouped("group", "b", &["beta"]),
        grouped("group", "c", &["gamma"]),
    ];
    let world = World::new(&rows);
    let principal = grants(&["beta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
}

#[test]
fn auth_metadata_precedence_prefers_the_request_metadata_principal() {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "b", &["beta"]),
    ];
    let world = World::new(&rows);
    let direct = grants(&["group"], &[]);
    let grouped_only = grants(&["beta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&direct),
        litellm_metadata: Some(&grouped_only),
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: None,
        litellm_metadata: Some(&grouped_only),
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["b"]
    );
}

#[rstest]
#[case::no_metadata(None)]
#[case::no_grants(Some(KeyGrants::default()))]
fn no_principal_or_grants_leaves_the_candidates_untouched(#[case] metadata: Option<KeyGrants>) {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "b", &["beta"]),
    ];
    let world = World::new(&rows);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: metadata.as_ref(),
        litellm_metadata: None,
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
}

#[rstest]
#[case::the_model_itself(&["group"])]
#[case::the_wildcard(&["*"])]
#[case::the_proxy_wildcard(&["all-proxy-models"])]
#[case::a_team_grant(&[])]
fn direct_and_wildcard_grants_bypass_the_filter(#[case] models: &[&str]) {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "b", &["beta"]),
    ];
    let world = World::new(&rows);
    let team = if models.is_empty() {
        vec!["group"]
    } else {
        Vec::new()
    };
    let principal = grants(models, &team);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
}

#[test]
fn no_candidates_come_back_untouched() {
    let candidates: [DeploymentSpec; 0] = [];
    let world = World::new(&candidates);
    let principal = grants(&["beta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &candidates,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert!(filtered_ids(request, &AccessGroups::default(), &world).is_empty());
}

#[rstest]
#[case::the_model_itself("group")]
#[case::the_wildcard("*")]
#[case::the_proxy_wildcard("all-proxy-models")]
fn a_grant_named_like_an_access_group_still_bypasses_the_filter(#[case] token: &str) {
    let rows = [
        grouped("group", "a", &[token]),
        grouped("group", "b", &["alpha"]),
    ];
    let world = World::new(&rows);
    let principal = grants(&[token], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
}

#[test]
fn no_group_overlap_leaves_the_candidates_untouched() {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "b", &["alpha"]),
    ];
    let world = World::new(&rows);
    let principal = grants(&["delta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &rows,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert_eq!(
        filtered_ids(request, &AccessGroups::default(), &world),
        ["a", "b"]
    );
}

#[test]
fn group_only_result_can_be_empty() {
    let rows = [
        grouped("group", "a", &["alpha"]),
        grouped("group", "c", &["gamma"]),
    ];
    let world = World::new(&rows);
    let candidates = [rows.first().expect("the first candidate").clone()];
    let principal = grants(&["gamma"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &candidates,
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    assert!(filtered_ids(request, &AccessGroups::default(), &world).is_empty());
}

#[test]
fn an_unlisted_access_groups_value_is_rejected() {
    let mut broken = grouped("group", "a", &["beta"]);
    broken
        .model_info
        .insert("access_groups".to_owned(), text("beta"));
    let world = World::new(&[grouped("group", "a", &["beta"])]);
    let principal = grants(&["beta"], &[]);
    let request = AccessFilterRequest {
        model: "group",
        deployments: &[broken],
        metadata: Some(&principal),
        litellm_metadata: None,
        team_id: None,
    };
    let lookup = world.lookup();
    let outcome = filter_by_access_groups(request, &AccessGroups::default(), &lookup);
    assert!(matches!(
        outcome,
        Err(AccessGroupError::InvalidAccessGroups { model_name }) if model_name == "group"
    ));
}

fn filtered_ids(
    request: AccessFilterRequest<'_>,
    access_groups: &AccessGroups,
    world: &World,
) -> Vec<String> {
    let lookup = world.lookup();
    filter_by_access_groups(request, access_groups, &lookup)
        .expect("the access filter")
        .iter()
        .map(|deployment| deployment.id().unwrap_or_default().to_owned())
        .collect()
}

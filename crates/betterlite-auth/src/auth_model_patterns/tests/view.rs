use std::collections::BTreeMap;

use indexmap::IndexMap;

use crate::auth_model_patterns::view::{
    Deployment, DeploymentView, ModelAccessView, ModelGroupAlias,
};

fn listing_deployments() -> Vec<Deployment> {
    vec![
        Deployment {
            model_name: "gpt-4o".to_owned(),
            access_groups: vec!["group-a".to_owned()],
            ..Deployment::default()
        },
        Deployment {
            model_name: "alias-target".to_owned(),
            access_groups: vec!["group-b".to_owned()],
            ..Deployment::default()
        },
        Deployment {
            model_name: "model_name_team-a_1".to_owned(),
            access_groups: vec!["group-d".to_owned()],
            team_id: Some("team-a".to_owned()),
            team_public_model_name: Some("team-public".to_owned()),
        },
        Deployment {
            model_name: "team-public".to_owned(),
            access_groups: vec!["group-e".to_owned()],
            ..Deployment::default()
        },
    ]
}

fn listing_aliases() -> IndexMap<String, ModelGroupAlias> {
    IndexMap::from([
        (
            "shown".to_owned(),
            ModelGroupAlias {
                model: "alias-target".to_owned(),
                hidden: false,
            },
        ),
        (
            "hidden-alias".to_owned(),
            ModelGroupAlias {
                model: "gpt-4o".to_owned(),
                hidden: true,
            },
        ),
        (
            "empty".to_owned(),
            ModelGroupAlias {
                model: String::new(),
                hidden: false,
            },
        ),
    ])
}

// a router whose model list and alias map both name something
fn listing_view() -> DeploymentView {
    DeploymentView {
        deployments: listing_deployments(),
        model_group_alias: listing_aliases(),
    }
}

#[test]
fn deployment_view_rows_follow_router_listing() {
    let view = listing_view();

    // an alias row keeps its target deployment's access groups under the alias name
    assert_eq!(
        view.access_groups("shown", None),
        BTreeMap::from([("group-b".to_owned(), vec!["shown".to_owned()])])
    );
    // a team public name resolves to the team's own deployment row
    assert_eq!(
        view.access_groups("team-public", Some("team-a")),
        BTreeMap::from([("group-d".to_owned(), vec!["model_name_team-a_1".to_owned()])])
    );
    // the team's public name wins over a global deployment of the same name
    assert_eq!(
        view.deployed("team-public", Some("team-a"))[0].model_name,
        "model_name_team-a_1"
    );
    assert_eq!(
        view.access_groups("team-public", Some("team-b")),
        BTreeMap::from([("group-e".to_owned(), vec!["team-public".to_owned()])])
    );
    assert_eq!(view.deployed("team-public", None).len(), 1);
    assert_eq!(
        view.deployed("model_name_team-a_1", Some("team-a")).len(),
        1
    );
    assert!(
        view.deployed("model_name_team-a_1", Some("team-b"))
            .is_empty()
    );
}

// deployments that share a public name and deployments of other teams
fn team_rows_view() -> DeploymentView {
    DeploymentView {
        deployments: vec![
            Deployment {
                model_name: "model_name_team-a_1".to_owned(),
                access_groups: vec!["group-a".to_owned()],
                team_id: Some("team-a".to_owned()),
                team_public_model_name: Some("team-public".to_owned()),
            },
            Deployment {
                model_name: "model_name_team-b_9".to_owned(),
                access_groups: vec!["group-b".to_owned()],
                team_id: Some("team-b".to_owned()),
                team_public_model_name: Some("team-public".to_owned()),
            },
            Deployment {
                model_name: "model_name_team-a_2".to_owned(),
                access_groups: vec!["group-c".to_owned()],
                team_id: Some("team-a".to_owned()),
                team_public_model_name: Some("another-public".to_owned()),
            },
            Deployment {
                model_name: "gpt-4o".to_owned(),
                access_groups: vec!["group-d".to_owned()],
                ..Deployment::default()
            },
            Deployment {
                model_name: "shared".to_owned(),
                access_groups: vec!["group-e".to_owned()],
                team_id: Some("team-b".to_owned()),
                team_public_model_name: None,
            },
        ],
        model_group_alias: IndexMap::new(),
    }
}

#[test]
fn team_public_names_resolve_only_the_callers_row() {
    let view = team_rows_view();

    // only the caller's own row answers its public name
    let owned = view.deployed("team-public", Some("team-a"));
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].model_name, "model_name_team-a_1");
    assert_eq!(
        view.access_groups("team-public", Some("team-a")),
        BTreeMap::from([("group-a".to_owned(), vec!["model_name_team-a_1".to_owned()])])
    );
    // a public name no team of the caller's owns resolves to no row
    assert!(view.deployed("team-public", Some("team-c")).is_empty());
    // a public name the caller's team does not use resolves to no row either
    assert!(view.deployed("unowned-name", Some("team-a")).is_empty());
    // a name only another team deploys is not the caller's
    assert!(view.deployed("shared", Some("team-a")).is_empty());
    // the caller's own deployments stay reachable by their model name
    assert_eq!(
        view.deployed("gpt-4o", Some("team-a"))
            .iter()
            .map(|deployment| deployment.model_name.as_str())
            .collect::<Vec<_>>(),
        vec!["gpt-4o"]
    );
}

#[test]
fn deployment_view_lists_alias_targets_and_deployments() {
    let view = listing_view();

    assert_eq!(
        view.model_group_alias_target("shown").as_deref(),
        Some("alias-target")
    );
    assert!(view.model_group_alias_target("empty").is_none());
    assert!(view.has_deployment("model_name_team-a_1"));
    assert!(!view.has_deployment("missing"));

    // get_model_names: aliases first, then deployments, skipping team rows and hidden aliases
    assert_eq!(
        view.model_names(),
        vec![
            "shown".to_owned(),
            "gpt-4o".to_owned(),
            "alias-target".to_owned(),
            "team-public".to_owned(),
        ]
    );
}

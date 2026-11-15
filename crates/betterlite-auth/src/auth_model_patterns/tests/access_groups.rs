use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::auth_model_patterns::access_groups::{
    AccessGroupResources, ResourceField, collect_resources, models_from_access_groups,
};
use crate::auth_model_patterns::decision::{can_object_call_model, check_model_access};
use crate::auth_model_patterns::view::{DeploymentView, ModelAccessView as _};

use super::{FixedGroups, context, deployment, empty_aliases, empty_provider, grant, one};

fn known_lookup() -> FixedGroups {
    FixedGroups(BTreeMap::from([(
        "known-group".to_owned(),
        AccessGroupResources {
            model_names: vec!["grouped-model".to_owned()],
            mcp_server_ids: vec!["mcp-1".to_owned()],
            agent_ids: vec!["agent-1".to_owned()],
        },
    )]))
}

// a router whose one deployment carries an access group
fn grouped_view() -> DeploymentView {
    let mut view = DeploymentView::default();
    let mut grouped = deployment("grouped-model");
    grouped.access_groups = vec!["known-group".to_owned()];
    view.deployments.push(grouped);
    view
}

#[test]
fn unknown_group() {
    let view = grouped_view();
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(Some(&view), &aliases, &provider);

    let groups = view.access_groups("grouped-model", None);
    assert_eq!(
        groups,
        BTreeMap::from([("known-group".to_owned(), vec!["grouped-model".to_owned()])])
    );
    // a token whose models name an access group is granted that group and what it serves
    assert!(check_model_access(
        "grouped-model",
        &grant(&["known-group"]),
        &ctx
    ));
    // the group name itself is filtered out of the allowlist
    assert!(!check_model_access(
        "other-model",
        &grant(&["known-group"]),
        &ctx
    ));

    // a group that no view serves grants nothing
    assert!(!check_model_access(
        "grouped-model",
        &grant(&["unknown-group"]),
        &ctx
    ));
}

#[test]
fn no_access_group_store_keeps_the_denial() {
    let view = DeploymentView::default();
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(Some(&view), &aliases, &provider);
    // an unloadable group leaves only the allowlist, which does not hold the model
    assert!(
        !can_object_call_model(
            &one("unlisted"),
            &grant(&["gpt-4o"]),
            ModelAccessObject::Key,
            &ctx,
            0
        )
        .is_allowed()
    );
}

#[test]
fn access_group_resources_come_from_the_lookup() {
    // a helper with no access-group store loads no resources
    assert!(models_from_access_groups(&["known-group".to_owned()], None).is_empty());
    let lookup = known_lookup();
    assert_eq!(
        models_from_access_groups(&["known-group".to_owned()], Some(&lookup)),
        vec!["grouped-model".to_owned()]
    );
    // an unknown id is skipped unless the caller asked for the database only
    assert_eq!(
        models_from_access_groups(
            &["known-group".to_owned(), "missing".to_owned()],
            Some(&lookup)
        ),
        vec!["grouped-model".to_owned()]
    );
    let error = collect_resources(
        &["missing".to_owned()],
        ResourceField::ModelNames,
        Some(&lookup),
        true,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the access group missing could not be loaded"
    );
}

#[test]
fn collect_resources_answers_every_resource_field() {
    let lookup = known_lookup();
    // the same collector answers the MCP server and agent id fields
    assert_eq!(
        collect_resources(
            &["known-group".to_owned()],
            ResourceField::McpServerIds,
            Some(&lookup),
            false
        )
        .unwrap(),
        vec!["mcp-1".to_owned()]
    );
    assert_eq!(
        collect_resources(
            &["known-group".to_owned()],
            ResourceField::AgentIds,
            Some(&lookup),
            true
        )
        .unwrap(),
        vec!["agent-1".to_owned()]
    );
    assert!(
        collect_resources(&[], ResourceField::AgentIds, None, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn grouped_model_keeps_the_plain_allowlist() {
    let view = grouped_view();
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(Some(&view), &aliases, &provider);

    // the grouped deployment filters nothing out of a wildcard allowlist
    assert!(check_model_access(
        "grouped-model",
        &grant(&["grouped-*"]),
        &ctx
    ));
    assert!(!check_model_access(
        "other-model",
        &grant(&["grouped-*"]),
        &ctx
    ));
}

#[test]
fn access_group_resources_keep_first_seen_order() {
    // resources of several groups keep the first-seen order and drop duplicates
    let second = FixedGroups(BTreeMap::from([
        (
            "one".to_owned(),
            AccessGroupResources {
                model_names: vec!["a".to_owned(), "b".to_owned()],
                mcp_server_ids: Vec::new(),
                agent_ids: Vec::new(),
            },
        ),
        (
            "two".to_owned(),
            AccessGroupResources {
                model_names: vec!["b".to_owned(), "c".to_owned()],
                mcp_server_ids: Vec::new(),
                agent_ids: Vec::new(),
            },
        ),
    ]));
    assert_eq!(
        models_from_access_groups(&["one".to_owned(), "two".to_owned()], Some(&second)),
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
    );
}

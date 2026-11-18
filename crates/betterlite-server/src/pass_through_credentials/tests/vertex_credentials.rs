#![cfg(test)]

use rstest::rstest;

use super::{Json, available, deployment, obj, router, text, vertex};

#[test]
fn a_named_model_returns_its_own_deployment() {
    let resolver = router(
        vec![
            vertex("first", "project-one", "us-central1"),
            vertex("second", "project-two", "us-central1"),
        ],
        &[],
    );
    let named = resolver
        .vertex_deployment_credentials(Some("second"))
        .expect("second project");
    assert_eq!(named.vertex_project.as_deref(), Some("project-two"));
    assert!(resolver.vertex_deployment_credentials(None).is_none());
}

#[test]
fn unanimous_deployments_give_the_single_default() {
    let resolver = router(
        vec![
            vertex("first", "shared-project", "us-central1"),
            vertex("second", "shared-project", "us-central1"),
        ],
        &[],
    );
    let unnamed = resolver
        .vertex_deployment_credentials(None)
        .expect("unanimous default");
    assert_eq!(unnamed.vertex_project.as_deref(), Some("shared-project"));
    assert_eq!(unnamed.vertex_location.as_deref(), Some("us-central1"));
}

#[test]
fn disagreeing_deployments_return_none() {
    let resolver = router(
        vec![
            vertex("first", "project-one", "us-central1"),
            vertex("second", "project-two", "us-central1"),
        ],
        &[],
    );
    assert!(resolver.vertex_deployment_credentials(None).is_none());
}

#[test]
fn a_deployment_without_project_or_location_is_ignored() {
    let incomplete = available(
        "incomplete",
        "vertex_ai/incomplete",
        &[("vertex_project", text("project-one"))],
    );
    let complete = vertex("complete", "project-one", "us-central1");
    let resolver = router(vec![incomplete, complete], &[]);
    let unnamed = resolver
        .vertex_deployment_credentials(None)
        .expect("the complete deployment");
    assert_eq!(unnamed.vertex_project.as_deref(), Some("project-one"));
}

#[test]
fn a_non_vertex_deployment_is_ignored() {
    let resolver = router(vec![available("gpt", "openai/gpt-4o-mini", &[])], &[]);
    assert!(resolver.vertex_deployment_credentials(None).is_none());
}
#[rstest]
#[case::model("model")]
#[case::project("vertex_project")]
#[case::location("vertex_location")]
#[case::enabled("use_in_pass_through")]
fn incomplete_and_disabled_vertex_deployments_never_answer(#[case] field: &str) {
    let mut params = obj(&[
        ("model", text("vertex_ai/only")),
        ("vertex_project", text("project")),
        ("vertex_location", text("location")),
        ("use_in_pass_through", Json::Bool(true)),
    ]);
    params.shift_remove(field);
    let resolver = router(vec![deployment("only", &params)], &[]);
    assert!(
        resolver
            .vertex_deployment_credentials(Some("only"))
            .is_none()
    );
}

#[test]
fn an_unknown_model_uses_only_a_unanimous_identity() {
    let mut resolver = router(vec![vertex("only", "project", "location")], &[]);
    assert_eq!(
        resolver
            .vertex_deployment_credentials(Some("unknown"))
            .unwrap()
            .vertex_project
            .as_deref(),
        Some("project")
    );
    resolver
        .deployments
        .push(vertex("second", "project", "elsewhere"));
    assert!(
        resolver
            .vertex_deployment_credentials(Some("unknown"))
            .is_none()
    );
}

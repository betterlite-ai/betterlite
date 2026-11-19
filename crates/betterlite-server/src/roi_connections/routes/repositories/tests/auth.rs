#![cfg(test)]

use axum::http::StatusCode;
use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use crate::roi_connections::access::READ_FORBIDDEN;
use crate::roi_connections::routes::repositories::REPOSITORIES_PATH;
use crate::roi_connections::tests::support::{
    Answer, admin, call, caller, get, harness, master, member,
};

#[rstest]
#[case::admin("user", vec![Role::PlatformAdmin])]
#[case::viewer("user", vec![Role::PlatformViewer])]
#[case::admin_and_viewer("user", vec![Role::PlatformAdmin, Role::PlatformViewer])]
#[tokio::test]
async fn read_admin_allows_the_proxy_admin_roles(#[case] subject: &str, #[case] roles: Vec<Role>) {
    let harness = harness().answers(vec![Answer::ok("[{\"full_name\":\"acme/widgets\"}]")]);
    let context = caller(subject, &roles);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&context))).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn read_admin_allows_the_master_key() {
    let harness = harness().answers(vec![Answer::ok("[]")]);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&master()))).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn read_admin_refuses_a_non_admin_with_403() {
    let harness = harness();
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&member()))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), READ_FORBIDDEN);
}

#[tokio::test]
async fn read_admin_without_a_caller_is_unauthenticated() {
    let harness = harness();
    let reply = call(harness.app(), get(REPOSITORIES_PATH, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(reply.text().contains("auth_error"));
}

#[tokio::test]
async fn read_admin_without_injection_reads_through_the_shared_transport() {
    let harness = harness().answers(vec![Answer::ok("[{\"full_name\":\"acme/widgets\"}]")]);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(harness.wire.seen().len(), 1);
}

#[rstest]
#[case::org_admin(vec![Role::OrgAdmin])]
#[case::org_viewer(vec![Role::OrgViewer])]
#[case::team_admin(vec![Role::TeamAdmin])]
#[case::team_member(vec![Role::TeamMember])]
#[case::no_roles(Vec::new())]
#[tokio::test]
async fn non_admin_callers_are_refused(#[case] roles: Vec<Role>) {
    let harness = harness();
    let context = caller("user", &roles);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&context))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), READ_FORBIDDEN);
}

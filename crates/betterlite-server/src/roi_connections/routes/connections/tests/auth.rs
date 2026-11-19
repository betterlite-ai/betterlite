#![cfg(test)]

use axum::http::StatusCode;
use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use crate::roi_connections::access::WRITE_FORBIDDEN;
use crate::roi_connections::routes::connections::CONNECTIONS_TEST_PATH;
use crate::roi_connections::tests::support::{admin, call, caller, harness, master, post, viewer};

// an empty model catalog leaves the settings not ready, so the handler's own 409 shows the guard passed
#[tokio::test]
async fn write_admin_allows_the_platform_admin() {
    let harness = harness();
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn write_admin_allows_the_master_key() {
    let harness = harness();
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&master()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn write_admin_refuses_the_read_only_viewer() {
    let harness = harness();
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&viewer()))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
}

#[rstest]
#[case::org_admin(vec![Role::OrgAdmin])]
#[case::team_member(vec![Role::TeamMember])]
#[case::no_roles(Vec::new())]
#[tokio::test]
async fn write_admin_refuses_a_non_admin(#[case] roles: Vec<Role>) {
    let harness = harness();
    let context = caller("user", &roles);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&context))).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_FORBIDDEN);
}

#[tokio::test]
async fn write_admin_without_a_caller_is_unauthenticated() {
    let harness = harness();
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

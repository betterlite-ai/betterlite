#![cfg(test)]

mod support;

use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_logging::key_invitation_email::link::InvitationCreator as _;
use chrono::TimeDelta;
use uuid::Uuid;

use super::{InvitationError, InvitationLinks, create_invitation_for_user};
use support::Database;

#[tokio::test]
async fn invitation_row_expires_seven_days_after_creation() {
    let database = Database::start().await;
    database.insert_users(&["invitee", "admin"]).await;
    let store = database.store();
    let record =
        create_invitation_for_user(Some(&store), "invitee", Some("admin"), "default_user_id")
            .await
            .expect("the invitation is created");
    assert_eq!(record.expires_at - record.created_at, TimeDelta::days(7));
    assert_eq!(record.created_at, record.updated_at);
    let stored = database
        .stored_invitation(&record.id)
        .await
        .expect("the stored row");
    assert_eq!(record.created_at, stored.created_at.and_utc());
    assert_eq!(record.expires_at, stored.expires_at.and_utc());
    assert_eq!(record.updated_at, stored.updated_at.and_utc());
    database.close().await;
}

#[tokio::test]
async fn invitation_row_takes_the_stored_defaults() {
    let database = Database::start().await;
    database.insert_users(&["invitee", "admin"]).await;
    let store = database.store();
    let record =
        create_invitation_for_user(Some(&store), "invitee", Some("admin"), "default_user_id")
            .await
            .expect("the invitation is created");
    let parsed = Uuid::parse_str(&record.id).expect("a uuid");
    assert_eq!(parsed.get_version_num(), 4);
    assert!(!record.is_accepted);
    assert_eq!(record.accepted_at, None);
    let stored = database
        .stored_invitation(&record.id)
        .await
        .expect("the stored row");
    assert!(!stored.is_accepted);
    assert_eq!(stored.accepted_at, None);
    database.close().await;
}

#[tokio::test]
async fn invitation_actor_is_the_caller_when_set() {
    let database = Database::start().await;
    database.insert_users(&["invitee", "admin"]).await;
    let store = database.store();
    let record =
        create_invitation_for_user(Some(&store), "invitee", Some("admin"), "default_user_id")
            .await
            .expect("the invitation is created");
    assert_eq!(record.created_by, "admin");
    assert_eq!(record.updated_by, "admin");
    database.close().await;
}

#[tokio::test]
async fn invitation_actor_falls_back_for_an_empty_caller() {
    let database = Database::start().await;
    database.insert_users(&["invitee", "default_user_id"]).await;
    let store = database.store();
    let record = create_invitation_for_user(Some(&store), "invitee", Some(""), "default_user_id")
        .await
        .expect("the invitation is created");
    assert_eq!(record.created_by, "default_user_id");
    assert_eq!(record.updated_by, "default_user_id");
    database.close().await;
}

#[tokio::test]
async fn invitation_actor_falls_back_for_a_missing_caller() {
    let database = Database::start().await;
    database.insert_users(&["invitee", "default_user_id"]).await;
    let store = database.store();
    let record = create_invitation_for_user(Some(&store), "invitee", None, "default_user_id")
        .await
        .expect("the invitation is created");
    assert_eq!(record.created_by, "default_user_id");
    assert_eq!(record.updated_by, "default_user_id");
    database.close().await;
}

#[tokio::test]
async fn invitation_without_a_store_is_a_bad_request() {
    let error = create_invitation_for_user(None, "invitee", Some("admin"), "default_user_id")
        .await
        .expect_err("the database is missing");
    assert!(matches!(error, InvitationError::NoDatabase));
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.message(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[tokio::test]
async fn invitation_for_an_unknown_invitee_is_a_bad_request() {
    let database = Database::start().await;
    database.insert_users(&["admin"]).await;
    let store = database.store();
    let error = create_invitation_for_user(Some(&store), "ghost", Some("admin"), "default_user_id")
        .await
        .expect_err("the invitee has no user row");
    assert!(matches!(error, InvitationError::UserMissing));
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.message(),
        "User id does not exist in 'LiteLLM_UserTable'. Fix this by creating user via `/user/new`."
    );
    assert_eq!(database.invitation_count().await, 0);
    database.close().await;
}

#[tokio::test]
async fn invitation_from_an_unknown_actor_is_a_bad_request() {
    let database = Database::start().await;
    database.insert_users(&["invitee"]).await;
    let store = database.store();
    let error =
        create_invitation_for_user(Some(&store), "invitee", Some("ghost"), "default_user_id")
            .await
            .expect_err("the actor has no user row");
    assert!(matches!(error, InvitationError::UserMissing));
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.message(),
        "User id does not exist in 'LiteLLM_UserTable'. Fix this by creating user via `/user/new`."
    );
    assert_eq!(database.invitation_count().await, 0);
    database.close().await;
}

#[tokio::test]
async fn invitation_insert_failure_is_an_internal_error() {
    let database = Database::start().await;
    database.insert_users(&["invitee", "admin"]).await;
    database.run(r#"DROP TABLE "LiteLLM_InvitationLink""#).await;
    let driver_message = database.probe_insert_error_message().await;
    let store = database.store();
    let error =
        create_invitation_for_user(Some(&store), "invitee", Some("admin"), "default_user_id")
            .await
            .expect_err("the insert fails");
    assert!(matches!(error, InvitationError::Insert(_)));
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error.message(), driver_message);
    database.close().await;
}

#[tokio::test]
async fn invitation_port_creates_the_link_for_the_recipient() {
    let database = Database::start().await;
    database.insert_users(&["recipient"]).await;
    let links = InvitationLinks {
        store: database.store(),
        proxy_admin_name: "default_user_id".to_owned(),
    };
    let invitation = links
        .create_for_user("recipient")
        .await
        .expect("the link is created");
    let stored = database
        .stored_invitation(&invitation.0)
        .await
        .expect("the stored row");
    assert_eq!(stored.id, invitation.0);
    assert_eq!(stored.user_id, "recipient");
    assert_eq!(stored.created_by, "recipient");
    assert_eq!(stored.updated_by, "recipient");
    database.close().await;
}

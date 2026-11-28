use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::TestDatabase;
use crate::auth::auth_key_lookup::tests::{caller_through_key_stage, master_caller};
use crate::organization_create::access::{
    DatabaseUserLookup, NO_ACCESS_DETAIL, OrgAccessError, OrgUserLookup, verify_org_access,
};
use crate::organization_create::caller::OrgCaller;

fn caller(user_id: Option<&str>, admin_view: bool) -> OrgCaller {
    OrgCaller {
        user_id: user_id.map(str::to_owned),
        user_role: None,
        models: Vec::new(),
        admin_view,
    }
}

async fn rendered(error: OrgAccessError) -> (StatusCode, String) {
    let response: Response = error.into_response();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

struct MissingUser;

#[async_trait]
impl OrgUserLookup for MissingUser {
    async fn find_user(&self, _user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
        Ok(None)
    }
}

struct FailingLookup;

#[async_trait]
impl OrgUserLookup for FailingLookup {
    async fn find_user(&self, _user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
        Err(OrgAccessError::Session(DbError::NoDbConnected))
    }
}

struct PanicLookup;

#[async_trait]
impl OrgUserLookup for PanicLookup {
    async fn find_user(&self, _user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
        panic!("an administrator must pass without the user lookup");
    }
}

#[test]
fn org_caller_of_a_key_whose_user_is_master_key_is_not_an_admin() {
    let spoofed = caller_through_key_stage(r#"{"user_id":"master_key"}"#);
    let spoof = OrgCaller::from_principal(&spoofed.principal).expect("the key projects");
    assert_eq!(spoof.user_id.as_deref(), Some("master_key"));
    assert_eq!(spoof.user_role, None);
    assert!(!spoof.admin_view);
    let master =
        OrgCaller::from_principal(&master_caller().principal).expect("the record projects");
    assert_eq!(master.user_id.as_deref(), Some("default_user_id"));
    assert_eq!(master.user_role, Some(UserRole::ProxyAdmin));
    assert!(master.admin_view);
}

#[tokio::test]
async fn org_access_admin_view_bypasses_lookup() {
    verify_org_access("org-1", &caller(Some("u1"), true), &PanicLookup)
        .await
        .unwrap();
    verify_org_access("org-1", &caller(None, true), &PanicLookup)
        .await
        .unwrap();
}

#[tokio::test]
async fn org_access_missing_user() {
    let missing = verify_org_access("org-1", &caller(Some("u1"), false), &MissingUser).await;
    assert!(matches!(missing, Err(OrgAccessError::Denied)));
    // a caller without a user id is refused before any lookup
    let anonymous = verify_org_access("org-1", &caller(None, false), &PanicLookup).await;
    assert!(matches!(anonymous, Err(OrgAccessError::Denied)));

    let (status, text) = rendered(OrgAccessError::Denied).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        text,
        format!(r#"{{"detail":"{NO_ACCESS_DETAIL}"}}"#),
        "the refusal carries a string detail"
    );
}

#[rstest]
#[case::exact_org_admin("u1", "org-1", true)]
#[case::other_org_admin_membership("u1", "org-2", false)]
#[case::wrong_role("u1", "org-3", false)]
#[case::null_role("u1", "org-4", false)]
#[case::no_memberships("u2", "org-1", false)]
#[case::unknown_user("ghost", "org-1", false)]
#[tokio::test]
async fn org_access_membership_matrix(
    #[case] user_id: &str,
    #[case] organization_id: &str,
    #[case] allowed: bool,
) {
    let database = TestDatabase::start().await;
    database.seed_user("u1", None, &[]).await;
    database.seed_user("u2", None, &[]).await;
    database
        .seed_membership("u1", "org-1", Some("org_admin"))
        .await;
    database
        .seed_membership("u1", "org-3", Some("internal_user"))
        .await;
    database.seed_membership("u1", "org-4", None).await;
    let store = database.store();
    let lookup = DatabaseUserLookup {
        store: store.as_ref(),
    };
    let granted = verify_org_access(organization_id, &caller(Some(user_id), false), &lookup).await;
    assert_eq!(granted.is_ok(), allowed);
    database.close().await;
}

#[tokio::test]
async fn org_access_lookup_error() {
    let failed = verify_org_access("org-1", &caller(Some("u1"), false), &FailingLookup).await;
    let Err(OrgAccessError::Session(_)) = failed else {
        panic!("a failed lookup is reported, not hidden");
    };

    let (status, text) = rendered(OrgAccessError::Session(DbError::NoDbConnected)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let JsonData::Object(body) = serde_json::from_str(&text).unwrap() else {
        panic!("the internal error body is an object");
    };
    let Some(JsonData::Object(error)) = body.get("error") else {
        panic!("the internal error body carries error: {text}");
    };
    assert_eq!(
        error.get("message"),
        Some(&JsonData::String(String::from("Internal server error")))
    );
    assert_eq!(
        error.get("type"),
        Some(&JsonData::String(String::from("internal_server_error")))
    );

    let (db_status, db_text) = rendered(OrgAccessError::DatabaseNotConnected).await;
    assert_eq!(db_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        db_text,
        format!(
            r#"{{"detail":{{"error":"{}"}}}}"#,
            CommonProxyErrors::DbNotConnectedError.as_str()
        )
    );
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use rstest::rstest;

use crate::resolved_key::auth::{AuthError, UserAPIKeyAuth};
use crate::user_roles::UserRole;

use super::{
    AdmittedUserError, AdmittedUserReload, DelegatedHuman, ReadFault, TeamLimits, UserReads,
    with_session_resource_server_id,
};

const ROW: &str = r#"{"user_id":"test-user","user_role":"internal_user","organization_id":"test-org","object_permission_id":"test-permission","tpm_limit":100,"rpm_limit":10}"#;
const PERMISSION: &str =
    r#"{"object_permission_id":"test-permission","mcp_servers":["test-server"]}"#;

fn row(fields: &str) -> LiteLLMUserTable {
    serde_json::from_str(fields).unwrap()
}

fn permission(fields: &str) -> LiteLLMObjectPermissionTable {
    serde_json::from_str(fields).unwrap()
}

fn limits() -> BTreeMap<String, BTreeMap<String, i64>> {
    BTreeMap::from([(
        "test-team".to_owned(),
        BTreeMap::from([("test-server".to_owned(), 7)]),
    )])
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReadCall {
    User {
        user_id: String,
        fresh: bool,
    },
    Permission {
        object_permission_id: String,
        fresh: bool,
    },
}

struct FakeUsers {
    has_database: bool,
    user: Result<Option<LiteLLMUserTable>, ReadFault>,
    permission: Result<Option<LiteLLMObjectPermissionTable>, ReadFault>,
    calls: Mutex<Vec<ReadCall>>,
}

fn fake_users(
    user: Result<Option<LiteLLMUserTable>, ReadFault>,
    permission: Result<Option<LiteLLMObjectPermissionTable>, ReadFault>,
) -> FakeUsers {
    FakeUsers {
        has_database: true,
        user,
        permission,
        calls: Mutex::new(Vec::new()),
    }
}

impl FakeUsers {
    fn calls(&self) -> Vec<ReadCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl UserReads for FakeUsers {
    fn has_database(&self) -> bool {
        self.has_database
    }

    fn user_by_id(
        &self,
        user_id: &str,
        fresh: bool,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, ReadFault>> + Send {
        self.calls.lock().unwrap().push(ReadCall::User {
            user_id: user_id.to_owned(),
            fresh,
        });
        ready(self.user.clone())
    }

    fn object_permission(
        &self,
        object_permission_id: &str,
        fresh: bool,
    ) -> impl Future<Output = Result<Option<LiteLLMObjectPermissionTable>, ReadFault>> + Send {
        self.calls.lock().unwrap().push(ReadCall::Permission {
            object_permission_id: object_permission_id.to_owned(),
            fresh,
        });
        ready(self.permission.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SeenAuth {
    user_id: Option<String>,
    team_id: Option<String>,
    admitted: bool,
    requires_fresh: bool,
}

struct FakeTeamLimits {
    limits: Option<BTreeMap<String, BTreeMap<String, i64>>>,
    seen: Mutex<Vec<SeenAuth>>,
}

fn fake_team_limits(limits: Option<BTreeMap<String, BTreeMap<String, i64>>>) -> FakeTeamLimits {
    FakeTeamLimits {
        limits,
        seen: Mutex::new(Vec::new()),
    }
}

impl FakeTeamLimits {
    fn seen(&self) -> Vec<SeenAuth> {
        self.seen.lock().unwrap().clone()
    }
}

impl TeamLimits for FakeTeamLimits {
    fn team_rpm_limits(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Option<BTreeMap<String, BTreeMap<String, i64>>>> + Send {
        self.seen.lock().unwrap().push(SeenAuth {
            user_id: auth.view.token.user_id.clone(),
            team_id: auth.view.token.team_id.clone(),
            admitted: auth.extra.mcp_access.mcp_admitted_user_subject,
            requires_fresh: auth.extra.requires_fresh_policy,
        });
        ready(self.limits.clone())
    }
}

fn build_reloader(
    users: FakeUsers,
    team_limits: FakeTeamLimits,
) -> AdmittedUserReload<FakeUsers, FakeTeamLimits> {
    AdmittedUserReload { users, team_limits }
}

fn status_and_detail(outcome: Result<UserAPIKeyAuth, AdmittedUserError>) -> (u16, String) {
    let error = outcome.unwrap_err();
    (error.status(), error.to_string())
}

#[rstest]
#[case::no_database(
    AdmittedUserError::NoDatabase,
    500,
    "Server misconfigured: no database connection"
)]
#[case::invalid_credential(
    AdmittedUserError::InvalidCredential,
    401,
    "Invalid or expired credential"
)]
#[case::database_unavailable(
    AdmittedUserError::DatabaseUnavailable {
        message: "the auth database is unavailable".to_owned(),
    },
    503,
    "the auth database is unavailable"
)]
fn error_status_and_detail_match_litellm(
    #[case] error: AdmittedUserError,
    #[case] status: u16,
    #[case] detail: &str,
) {
    assert_eq!(error.status(), status);
    assert_eq!(error.to_string(), detail);
}

#[tokio::test]
async fn error_a_user_role_that_is_not_a_user_role_is_a_500_that_keeps_the_auth_error() {
    let users = fake_users(
        Ok(Some(row(
            r#"{"user_id":"test-user","user_role":"test-role"}"#,
        ))),
        Ok(None),
    );
    let reloader = build_reloader(users, fake_team_limits(None));
    let error = reloader.reload("test-user", false).await.unwrap_err();
    assert_eq!(error.status(), 500);
    let source = error.source().expect("the auth error stays the source");
    assert!(source.downcast_ref::<AuthError>().is_some());
    assert_eq!(error.to_string(), source.to_string());
}

#[tokio::test]
async fn reload_without_a_database_gives_no_database_before_any_read() {
    let users = FakeUsers {
        has_database: false,
        user: Ok(Some(row(ROW))),
        permission: Ok(None),
        calls: Mutex::new(Vec::new()),
    };
    let reloader = build_reloader(users, fake_team_limits(None));
    let (status, detail) = status_and_detail(reloader.reload("test-user", false).await);
    assert_eq!(
        (status, detail.as_str()),
        (500, "Server misconfigured: no database connection")
    );
    assert!(reloader.users.calls().is_empty());
}

#[rstest]
#[case::fresh(false)]
#[case::stale(true)]
#[tokio::test]
async fn reload_reads_the_user_with_the_fresh_flag_it_was_given(#[case] fresh: bool) {
    let users = fake_users(Ok(Some(row(ROW))), Ok(None));
    let reloader = build_reloader(users, fake_team_limits(None));
    reloader.reload("test-subject", fresh).await.unwrap();
    assert_eq!(
        reloader.users.calls(),
        vec![ReadCall::User {
            user_id: "test-subject".to_owned(),
            fresh,
        }]
    );
}

#[rstest]
#[case::fresh(false)]
#[case::stale(true)]
#[tokio::test]
async fn reload_loads_a_missing_object_permission_with_the_same_fresh_flag(#[case] fresh: bool) {
    let users = fake_users(Ok(Some(row(ROW))), Ok(Some(permission(PERMISSION))));
    let reloader = build_reloader(users, fake_team_limits(None));
    let auth = reloader.reload("test-user", fresh).await.unwrap();
    assert_eq!(
        reloader.users.calls(),
        vec![
            ReadCall::User {
                user_id: "test-user".to_owned(),
                fresh,
            },
            ReadCall::Permission {
                object_permission_id: "test-permission".to_owned(),
                fresh,
            },
        ]
    );
    assert_eq!(
        auth.view.token.object_permission,
        Some(permission(PERMISSION))
    );
}

#[tokio::test]
async fn reload_keeps_the_rows_own_object_permission_without_a_second_read() {
    let own = r#"{"user_id":"test-user","object_permission_id":"test-permission","object_permission":{"object_permission_id":"test-own"}}"#;
    let users = fake_users(Ok(Some(row(own))), Ok(Some(permission(PERMISSION))));
    let reloader = build_reloader(users, fake_team_limits(None));
    let auth = reloader.reload("test-user", false).await.unwrap();
    assert_eq!(
        reloader.users.calls(),
        vec![ReadCall::User {
            user_id: "test-user".to_owned(),
            fresh: false,
        }]
    );
    assert_eq!(
        auth.view.token.object_permission,
        Some(permission(r#"{"object_permission_id":"test-own"}"#))
    );
}

#[tokio::test]
async fn reload_reads_no_object_permission_when_the_row_names_none() {
    let users = fake_users(
        Ok(Some(row(r#"{"user_id":"test-user"}"#))),
        Ok(Some(permission(PERMISSION))),
    );
    let reloader = build_reloader(users, fake_team_limits(None));
    let auth = reloader.reload("test-user", false).await.unwrap();
    assert_eq!(
        reloader.users.calls(),
        vec![ReadCall::User {
            user_id: "test-user".to_owned(),
            fresh: false,
        }]
    );
    assert_eq!(auth.view.token.object_permission, None);
}

#[tokio::test]
async fn reload_passes_a_missing_object_permission_row_as_no_permission() {
    let users = fake_users(Ok(Some(row(ROW))), Ok(None));
    let reloader = build_reloader(users, fake_team_limits(None));
    let auth = reloader.reload("test-user", false).await.unwrap();
    assert_eq!(
        reloader.users.calls(),
        vec![
            ReadCall::User {
                user_id: "test-user".to_owned(),
                fresh: false,
            },
            ReadCall::Permission {
                object_permission_id: "test-permission".to_owned(),
                fresh: false,
            },
        ]
    );
    assert_eq!(auth.view.token.object_permission, None);
}

#[rstest]
#[case::refused(ReadFault::Refused, 401, "Invalid or expired credential")]
#[case::other(ReadFault::Other("the row did not parse".to_owned()), 401, "Invalid or expired credential")]
#[case::outage(
    ReadFault::DatabaseUnavailable { message: "the auth database is unavailable".to_owned() },
    503,
    "the auth database is unavailable"
)]
#[tokio::test]
async fn reload_maps_a_user_read_fault_to_its_status(
    #[case] fault: ReadFault,
    #[case] status: u16,
    #[case] detail: &str,
) {
    let reloader = build_reloader(fake_users(Err(fault), Ok(None)), fake_team_limits(None));
    assert_eq!(
        status_and_detail(reloader.reload("test-user", false).await),
        (status, detail.to_owned())
    );
}

#[rstest]
#[case::refused(ReadFault::Refused, 401, "Invalid or expired credential")]
#[case::other(ReadFault::Other("the row did not parse".to_owned()), 401, "Invalid or expired credential")]
#[case::outage(
    ReadFault::DatabaseUnavailable { message: "the auth database is unavailable".to_owned() },
    503,
    "the auth database is unavailable"
)]
#[tokio::test]
async fn reload_maps_an_object_permission_fault_like_a_user_fault(
    #[case] fault: ReadFault,
    #[case] status: u16,
    #[case] detail: &str,
) {
    let users = fake_users(Ok(Some(row(ROW))), Err(fault));
    let reloader = build_reloader(users, fake_team_limits(None));
    assert_eq!(
        status_and_detail(reloader.reload("test-user", false).await),
        (status, detail.to_owned())
    );
}

#[tokio::test]
async fn reload_gives_401_for_a_missing_user_row() {
    let users = fake_users(Ok(None), Ok(Some(permission(PERMISSION))));
    let reloader = build_reloader(users, fake_team_limits(None));
    assert_eq!(
        status_and_detail(reloader.reload("test-user", false).await),
        (401, "Invalid or expired credential".to_owned())
    );
    assert_eq!(
        reloader.users.calls(),
        vec![ReadCall::User {
            user_id: "test-user".to_owned(),
            fresh: false,
        }]
    );
}

#[tokio::test]
async fn reload_gives_401_when_scim_active_is_json_false() {
    let users = fake_users(
        Ok(Some(row(
            r#"{"user_id":"test-user","metadata":{"scim_active":false}}"#,
        ))),
        Ok(None),
    );
    let reloader = build_reloader(users, fake_team_limits(None));
    assert_eq!(
        status_and_detail(reloader.reload("test-user", false).await),
        (401, "Invalid or expired credential".to_owned())
    );
}

#[rstest]
#[case::flag_true(r#"{"scim_active":true}"#)]
#[case::null(r#"{"scim_active":null}"#)]
#[case::zero(r#"{"scim_active":0}"#)]
#[case::text(r#"{"scim_active":"false"}"#)]
#[case::other_keys(r#"{"other":"value"}"#)]
#[tokio::test]
async fn reload_passes_metadata_whose_scim_active_is_not_json_false(#[case] metadata: &str) {
    let users = fake_users(
        Ok(Some(row(&format!(
            r#"{{"user_id":"test-user","metadata":{metadata}}}"#
        )))),
        Ok(None),
    );
    let reloader = build_reloader(users, fake_team_limits(None));
    reloader.reload("test-user", false).await.unwrap();
    assert_eq!(reloader.users.calls().len(), 1);
}

#[tokio::test]
async fn reload_passes_a_user_row_without_metadata() {
    let users = fake_users(Ok(Some(row(r#"{"user_id":"test-user"}"#))), Ok(None));
    let reloader = build_reloader(users, fake_team_limits(None));
    reloader.reload("test-user", false).await.unwrap();
}

#[tokio::test]
async fn reload_builds_the_admitted_auth_from_the_row() {
    let users = fake_users(Ok(Some(row(ROW))), Ok(Some(permission(PERMISSION))));
    let reloader = build_reloader(users, fake_team_limits(None));
    let auth = reloader.reload("test-user", false).await.unwrap();
    assert_eq!(auth.view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(auth.extra.user_role, Some(UserRole::InternalUser));
    assert_eq!(auth.view.token.org_id.as_deref(), Some("test-org"));
    assert_eq!(
        auth.view.token.object_permission_id.as_deref(),
        Some("test-permission")
    );
    assert_eq!(
        auth.view.token.object_permission,
        Some(permission(PERMISSION))
    );
    assert_eq!(auth.extra.user_tpm_limit, Some(100));
    assert_eq!(auth.extra.user_rpm_limit, Some(10));
    assert!(auth.view.token.token.is_none());
    assert!(auth.extra.api_key.0.is_none());
    assert!(auth.view.token.team_id.is_none());
}

#[tokio::test]
async fn reload_keeps_the_row_fields_it_does_not_have_absent() {
    let users = fake_users(Ok(Some(row(r#"{"user_id":"test-user"}"#))), Ok(None));
    let reloader = build_reloader(users, fake_team_limits(None));
    let auth = reloader.reload("test-user", false).await.unwrap();
    assert_eq!(auth.view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(auth.extra.user_role, None);
    assert_eq!(auth.view.token.org_id, None);
    assert_eq!(auth.view.token.object_permission_id, None);
    assert_eq!(auth.view.token.object_permission, None);
    assert_eq!(auth.extra.user_tpm_limit, None);
    assert_eq!(auth.extra.user_rpm_limit, None);
}

#[rstest]
#[case::fresh(false)]
#[case::stale(true)]
#[tokio::test]
async fn reload_stamps_the_admitted_marker_and_the_fresh_flag(#[case] fresh: bool) {
    let reloader = build_reloader(
        fake_users(Ok(Some(row(ROW))), Ok(None)),
        fake_team_limits(None),
    );
    let auth = reloader.reload("test-user", fresh).await.unwrap();
    assert!(auth.extra.mcp_access.mcp_admitted_user_subject);
    assert_eq!(auth.extra.requires_fresh_policy, fresh);
}

#[tokio::test]
async fn reload_hands_the_stamped_auth_to_team_limits_and_keeps_their_answer() {
    let reloader = build_reloader(
        fake_users(Ok(Some(row(ROW))), Ok(None)),
        fake_team_limits(Some(limits())),
    );
    let auth = reloader.reload("test-user", true).await.unwrap();
    assert_eq!(
        reloader.team_limits.seen(),
        vec![SeenAuth {
            user_id: Some("test-user".to_owned()),
            team_id: None,
            admitted: true,
            requires_fresh: true,
        }]
    );
    assert_eq!(auth.extra.mcp_source_team_rpm_limits, Some(limits()));
}

#[tokio::test]
async fn reload_stores_no_team_limits_when_the_port_returns_none() {
    let reloader = build_reloader(
        fake_users(Ok(Some(row(ROW))), Ok(None)),
        fake_team_limits(None),
    );
    let auth = reloader.reload("test-user", false).await.unwrap();
    assert_eq!(auth.extra.mcp_source_team_rpm_limits, None);
}

#[tokio::test]
async fn delegated_human_forwards_both_arguments() {
    let users = fake_users(Ok(Some(row(ROW))), Ok(None));
    let reloader = build_reloader(users, fake_team_limits(None));
    let auth = DelegatedHuman::reload_admitted_user(&reloader, "test-user", true)
        .await
        .unwrap();
    assert_eq!(auth.view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(
        reloader.users.calls(),
        vec![ReadCall::User {
            user_id: "test-user".to_owned(),
            fresh: true,
        }]
    );
}

#[tokio::test]
async fn delegated_human_returns_the_reload_error_unchanged() {
    let users = fake_users(Err(ReadFault::Refused), Ok(None));
    let reloader = build_reloader(users, fake_team_limits(None));
    let error = DelegatedHuman::reload_admitted_user(&reloader, "test-user", false)
        .await
        .unwrap_err();
    assert_eq!(error.status(), 401);
    assert_eq!(error.to_string(), "Invalid or expired credential");
}

#[rstest]
#[case::scoped(Some("test-server".to_owned()))]
#[case::aggregate(None)]
#[tokio::test]
async fn session_resource_setter_changes_only_the_resource_server_id(
    #[case] resource_server_id: Option<String>,
) {
    let reloader = build_reloader(
        fake_users(Ok(Some(row(ROW))), Ok(Some(permission(PERMISSION)))),
        fake_team_limits(Some(limits())),
    );
    let auth = reloader.reload("test-user", true).await.unwrap();
    let admitted = with_session_resource_server_id(auth, resource_server_id.clone());
    assert_eq!(
        admitted.extra.mcp_session_resource_server_id,
        resource_server_id
    );
    assert_eq!(admitted.view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(admitted.view.token.org_id.as_deref(), Some("test-org"));
    assert_eq!(
        admitted.view.token.object_permission,
        Some(permission(PERMISSION))
    );
    assert!(admitted.extra.mcp_access.mcp_admitted_user_subject);
    assert!(admitted.extra.requires_fresh_policy);
    assert_eq!(admitted.extra.mcp_source_team_rpm_limits, Some(limits()));
}

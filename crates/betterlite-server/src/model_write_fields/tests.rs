#![cfg(test)]

mod postgres;
mod support;

use std::sync::{Arc, Mutex};

use betterlite_auth::user_roles::UserRole;
use betterlite_db::team_repository::decoder::TeamRecordError;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_secrets::secret_codec::diagnostics::Level;
use rstest::rstest;

use self::support::{
    CountedOrgRoles, FakeTeams, RecordingPolicy, SEALED_NAME, caller, cipher, proxy_fields,
    rejection, team, team_access,
};
use crate::model_write_fields::access::{ModelCall, ModelWriteAccess, can_make_team_model_call};
use crate::model_write_fields::error::{
    AdminOnlyField, Detail, MemberCallError, ModelWriteError, Rejection,
};
use crate::model_write_fields::fields::{ModelPatch, can_attach_credential, can_set_session_tags};
use crate::model_write_fields::member::{MemberWrite, ModelWriteGrant};
use crate::team_access::error::TeamAccessError;

const ROLE: Option<UserRole> = Some(UserRole::InternalUser);
const ADMIN: Option<UserRole> = Some(UserRole::ProxyAdmin);

const fn label(error: &ModelWriteError) -> &'static str {
    match error {
        ModelWriteError::ViewOnly => "view_only",
        ModelWriteError::TeamMissing { .. } => "team_missing",
        ModelWriteError::OrphanedModel => "orphaned_model",
        ModelWriteError::NotTeamAdmin { .. } => "not_team_admin",
        ModelWriteError::NotProxyAdmin => "not_proxy_admin",
        ModelWriteError::MemberRouterUnavailable => "member_router_unavailable",
        ModelWriteError::AdminOnly { .. } => "admin_only",
        ModelWriteError::TeamRead(_) => "team_read",
        ModelWriteError::TeamAccess(_) => "team_access",
    }
}

fn outcome(result: Result<(), ModelWriteError>) -> Option<(&'static str, u16, &'static str)> {
    let error = result.err()?;
    let Rejection { status, detail } = rejection(&error);
    let shape = match detail {
        Detail::Message(_) => "message",
        Detail::Error(_) => "error",
        Detail::Proxy(_) => "proxy",
    };
    Some((label(&error), status.as_u16(), shape))
}

fn patch(json: &str) -> ModelPatch {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::proxy_admin_without_team(ADMIN, None, None)]
#[case::internal_user_without_team(ROLE, None, Some(("not_proxy_admin", 403, "error")))]
#[case::no_role_without_team(None, None, Some(("not_proxy_admin", 403, "error")))]
#[case::org_admin_without_team(Some(UserRole::OrgAdmin), None, Some(("not_proxy_admin", 403, "error")))]
#[case::proxy_viewer(Some(UserRole::ProxyAdminViewer), Some("team-1"), Some(("view_only", 403, "message")))]
#[case::internal_viewer(Some(UserRole::InternalUserViewer), None, Some(("view_only", 403, "message")))]
#[tokio::test]
async fn model_admin_write(
    #[case] role: Option<UserRole>,
    #[case] team_id: Option<&str>,
    #[case] expected: Option<(&'static str, u16, &'static str)>,
) {
    let teams = FakeTeams::holding(&["team-1"]);
    let access = team_access(&Arc::new(CountedOrgRoles::default()));
    let writes = ModelWriteAccess {
        teams: &teams,
        team_access: &access,
    };
    let result = writes
        .can_make_model_call(&caller("admin-1", role), team_id, ModelCall::Write)
        .await;
    assert_eq!(outcome(result), expected);
    assert!(teams.reads().is_empty());
}

#[rstest]
#[case::team_admin("admin-1", ROLE, "team-1", ModelCall::Write, None)]
#[case::roleless_team_admin("admin-1", None, "team-1", ModelCall::Write, None)]
#[case::member("member-1", ROLE, "team-1", ModelCall::Write, Some(("not_team_admin", 403, "error")))]
#[case::member_may_not_delete("member-1", ROLE, "team-1", ModelCall::Delete, Some(("not_team_admin", 403, "error")))]
#[case::proxy_admin_off_roster("outsider", ADMIN, "team-1", ModelCall::Write, None)]
#[case::missing_team("admin-1", ADMIN, "gone", ModelCall::Write, Some(("team_missing", 400, "error")))]
#[case::orphan_proxy_admin("outsider", ADMIN, "gone", ModelCall::Delete, None)]
#[case::orphan_team_admin("admin-1", ROLE, "gone", ModelCall::Delete, Some(("orphaned_model", 403, "error")))]
#[tokio::test]
async fn team_model_owner(
    #[case] user_id: &str,
    #[case] role: Option<UserRole>,
    #[case] team_id: &str,
    #[case] call: ModelCall,
    #[case] expected: Option<(&'static str, u16, &'static str)>,
) {
    let teams = FakeTeams::holding(&["team-1"]);
    let org_roles = Arc::new(CountedOrgRoles::default());
    let access = team_access(&org_roles);
    let writes = ModelWriteAccess {
        teams: &teams,
        team_access: &access,
    };
    let result = writes
        .can_make_model_call(&caller(user_id, role), Some(team_id), call)
        .await;
    assert_eq!(outcome(result), expected);
    assert_eq!(teams.reads(), vec![team_id.to_owned()]);
    assert!(org_roles.lookups.lock().unwrap().is_empty());
}

#[rstest]
#[case::no_team("member-1", ROLE, None, None, 0)]
#[case::team_admin("admin-1", ROLE, Some("team-1"), None, 1)]
#[case::viewer_on_roster("admin-1", Some(UserRole::ProxyAdminViewer), Some("team-1"), None, 1)]
#[case::proxy_admin_off_roster("outsider", ADMIN, Some("team-1"), None, 1)]
#[case::member("member-1", ROLE, Some("team-1"), Some(("not_team_admin", 403, "error")), 1)]
#[case::missing_team("outsider", ADMIN, Some("gone"), Some(("team_missing", 400, "error")), 1)]
#[tokio::test]
async fn team_model_action(
    #[case] user_id: &str,
    #[case] role: Option<UserRole>,
    #[case] team_id: Option<&str>,
    #[case] expected: Option<(&'static str, u16, &'static str)>,
    #[case] reads: usize,
) {
    let teams = FakeTeams::holding(&["team-1"]);
    let access = team_access(&Arc::new(CountedOrgRoles::default()));
    let writes = ModelWriteAccess {
        teams: &teams,
        team_access: &access,
    };
    let result = writes
        .can_make_model_call(&caller(user_id, role), team_id, ModelCall::TeamAction)
        .await;
    assert_eq!(outcome(result), expected);
    assert_eq!(teams.reads().len(), reads);
}

#[rstest]
#[case::proxy_admin_without_team("outsider", ADMIN, false, None)]
#[case::team_admin_without_team("admin-1", ROLE, false, Some(("not_team_admin", 403, "error")))]
#[case::team_admin("admin-1", ROLE, true, None)]
#[case::member("member-1", ROLE, true, Some(("not_team_admin", 403, "error")))]
fn team_model_call_needs_the_team(
    #[case] user_id: &str,
    #[case] role: Option<UserRole>,
    #[case] known: bool,
    #[case] expected: Option<(&'static str, u16, &'static str)>,
) {
    let stored = team("team-1");
    let found = known.then_some(&stored.fields.base);
    let result = can_make_team_model_call("team-1", &caller(user_id, role), found);
    assert_eq!(outcome(result), expected);
}

#[rstest]
#[case::no_params(None, None, ROLE, false, None)]
#[case::name_absent(
    Some("{}"),
    Some(r#"{"litellm_credential_name": "shared-openai"}"#),
    ROLE,
    true,
    None
)]
#[case::null_keeps_without_detach(
    Some(r#"{"litellm_credential_name": null}"#),
    Some(r#"{"litellm_credential_name": "shared-openai"}"#),
    ROLE,
    false,
    None
)]
#[case::null_detaches_stored(
    Some(r#"{"litellm_credential_name": null}"#),
    Some(r#"{"litellm_credential_name": "shared-openai"}"#),
    ROLE,
    true,
    Some(AdminOnlyField::DetachCredential)
)]
#[case::null_detaches_nothing(Some(r#"{"litellm_credential_name": null}"#), None, ROLE, true, None)]
#[case::null_over_stored_null(
    Some(r#"{"litellm_credential_name": null}"#),
    Some(r#"{"litellm_credential_name": null}"#),
    ROLE,
    true,
    None
)]
#[case::new_name(
    Some(r#"{"litellm_credential_name": "other"}"#),
    None,
    ROLE,
    false,
    Some(AdminOnlyField::AttachCredential)
)]
#[case::replaces_stored(
    Some(r#"{"litellm_credential_name": "other"}"#),
    Some(r#"{"litellm_credential_name": "shared-openai"}"#),
    ROLE,
    true,
    Some(AdminOnlyField::AttachCredential)
)]
#[case::same_plain_name(
    Some(r#"{"litellm_credential_name": "shared-openai"}"#),
    Some(r#"{"litellm_credential_name": "shared-openai"}"#),
    ROLE,
    false,
    None
)]
#[case::same_sealed_name(Some(r#"{"litellm_credential_name": "shared-openai"}"#), Some(r#"{"litellm_credential_name": "sERBFztHSkMcwAVhLYCuQALpnrBBFzqx1hX93kY7LtB8j8CSkMBDIDqJUHdGXfXY0LLBGkU="}"#), ROLE, false, None)]
#[case::sealed_text_itself(Some(r#"{"litellm_credential_name": "sERBFztHSkMcwAVhLYCuQALpnrBBFzqx1hX93kY7LtB8j8CSkMBDIDqJUHdGXfXY0LLBGkU="}"#), Some(r#"{"litellm_credential_name": "sERBFztHSkMcwAVhLYCuQALpnrBBFzqx1hX93kY7LtB8j8CSkMBDIDqJUHdGXfXY0LLBGkU="}"#), ROLE, false, Some(AdminOnlyField::AttachCredential))]
#[case::proxy_admin_attaches(
    Some(r#"{"litellm_credential_name": "other"}"#),
    None,
    ADMIN,
    false,
    None
)]
#[case::proxy_admin_detaches(
    Some(r#"{"litellm_credential_name": null}"#),
    Some(r#"{"litellm_credential_name": "shared-openai"}"#),
    ADMIN,
    true,
    None
)]
fn credential_attachment_denied(
    #[case] requested: Option<&str>,
    #[case] stored: Option<&str>,
    #[case] role: Option<UserRole>,
    #[case] null_detaches: bool,
    #[case] expected: Option<AdminOnlyField>,
) {
    let (incoming, existing) = (requested.map(patch), stored.map(patch));
    let result = can_attach_credential(
        incoming.as_ref(),
        existing.as_ref(),
        &caller("member-1", role),
        null_detaches,
        &cipher(),
    );
    let Some(field) = expected else {
        result.unwrap();
        return;
    };
    let error = result.unwrap_err();
    assert!(matches!(error, ModelWriteError::AdminOnly { field: found } if found == field));
    let fields = proxy_fields(&error);
    let wanted = (
        403,
        "auth_error".to_owned(),
        Some("litellm_credential_name".to_owned()),
        "403".to_owned(),
    );
    assert_eq!(fields, wanted);
}

#[rstest]
#[case::plain_name_logs_at_debug("shared-openai", 1)]
#[case::sealed_name_opens_quietly(SEALED_NAME, 0)]
fn stored_credential_names_decrypt(#[case] stored_name: &str, #[case] logged: usize) {
    let stored = ModelPatch {
        litellm_credential_name: Some(Some(stored_name.to_owned())),
        aws_session_tags: None,
    };
    let requested = ModelPatch {
        litellm_credential_name: Some(Some("shared-openai".to_owned())),
        aws_session_tags: None,
    };
    let sealed = cipher();
    can_attach_credential(
        Some(&requested),
        Some(&stored),
        &caller("member-1", ROLE),
        false,
        &sealed,
    )
    .unwrap();
    let entries = sealed.diagnostics.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), logged);
    assert!(entries.iter().all(|(level, message)| {
        *level == Level::Debug
            && message.contains("litellm_credential_name")
            && !message.contains(stored_name)
    }));
}

const TAGS: &str =
    r#"{"aws_session_tags": [{"Key": "team", "Value": "a"}, {"Key": "env", "Value": "prod"}]}"#;

#[rstest]
#[case::no_params(None, None, ROLE, false)]
#[case::no_tags(Some("{}"), Some(TAGS), ROLE, false)]
#[case::null_tags(Some(r#"{"aws_session_tags": null}"#), Some(TAGS), ROLE, false)]
#[case::proxy_admin_new_tags(Some(TAGS), None, ADMIN, false)]
#[case::member_new_tags(Some(TAGS), None, ROLE, true)]
#[case::member_over_null_tags(Some(TAGS), Some(r#"{"aws_session_tags": null}"#), ROLE, true)]
#[case::member_same_tags_reordered(
    Some(
        r#"{"aws_session_tags": [{"Key": "env", "Value": "prod"}, {"Key": "team", "Value": "a"}]}"#
    ),
    Some(TAGS),
    ROLE,
    false
)]
#[case::member_changed_value(
    Some(
        r#"{"aws_session_tags": [{"Key": "team", "Value": "b"}, {"Key": "env", "Value": "prod"}]}"#
    ),
    Some(TAGS),
    ROLE,
    true
)]
#[case::member_dropped_tag(
    Some(r#"{"aws_session_tags": [{"Key": "env", "Value": "prod"}]}"#),
    Some(TAGS),
    ROLE,
    true
)]
#[case::member_repeated_tag(Some(r#"{"aws_session_tags": [{"Key": "env", "Value": "prod"}, {"Key": "env", "Value": "prod"}]}"#), Some(r#"{"aws_session_tags": [{"Key": "env", "Value": "prod"}]}"#), ROLE, true)]
#[case::empty_over_empty(
    Some(r#"{"aws_session_tags": []}"#),
    Some(r#"{"aws_session_tags": []}"#),
    ROLE,
    false
)]
#[case::empty_over_nothing(Some(r#"{"aws_session_tags": []}"#), None, ROLE, true)]
fn session_tag_escalation(
    #[case] requested: Option<&str>,
    #[case] stored: Option<&str>,
    #[case] role: Option<UserRole>,
    #[case] denied: bool,
) {
    let (incoming, existing) = (requested.map(patch), stored.map(patch));
    let result = can_set_session_tags(
        incoming.as_ref(),
        existing.as_ref(),
        &caller("member-1", role),
    );
    if !denied {
        result.unwrap();
        return;
    }
    let error = result.unwrap_err();
    let fields = proxy_fields(&error);
    let wanted = (
        403,
        "auth_error".to_owned(),
        Some("aws_session_tags".to_owned()),
        "403".to_owned(),
    );
    assert_eq!(fields, wanted);
}

#[derive(Debug, Clone, Copy)]
enum Setup {
    Unregistered,
    Granting,
    Refusing,
}

#[rstest]
#[case::member_without_policy("member-1", ROLE, Some("team-1"), Setup::Unregistered, ("member_router_unavailable 400", 0))]
#[case::member_through_policy("member-1", ROLE, Some("team-1"), Setup::Granting, ("member update patch over stored", 1))]
#[case::member_refused("member-1", ROLE, Some("team-1"), Setup::Refusing, ("refused", 1))]
#[case::team_admin("admin-1", ROLE, Some("team-1"), Setup::Granting, ("authorized", 0))]
#[case::proxy_admin_off_roster("outsider", ADMIN, Some("team-1"), Setup::Unregistered, ("authorized", 0))]
#[case::proxy_admin_without_team("outsider", ADMIN, None, Setup::Unregistered, ("authorized", 0))]
#[case::member_without_team("member-1", ROLE, None, Setup::Granting, ("not_proxy_admin 403", 0))]
#[case::missing_team("member-1", ROLE, Some("gone"), Setup::Granting, ("team_missing 400", 0))]
#[case::viewer("member-1", Some(UserRole::InternalUserViewer), Some("team-1"), Setup::Granting, ("view_only 403", 0))]
#[tokio::test]
async fn member_router_policy(
    #[case] user_id: &str,
    #[case] role: Option<UserRole>,
    #[case] team_id: Option<&str>,
    #[case] setup: Setup,
    #[case] expected: (&str, usize),
) {
    let teams = FakeTeams::holding(&["team-1"]);
    let org_roles = Arc::new(CountedOrgRoles::default());
    let access = team_access(&org_roles);
    let writes = ModelWriteAccess {
        teams: &teams,
        team_access: &access,
    };
    let policy = RecordingPolicy {
        refuse: matches!(setup, Setup::Refusing),
        calls: Mutex::new(Vec::new()),
    };
    let registered = match setup {
        Setup::Unregistered => None,
        Setup::Granting | Setup::Refusing => Some(&policy),
    };
    let (incoming, existing) = ("patch".to_owned(), "stored".to_owned());
    let write = MemberWrite::Update {
        patch: &incoming,
        existing: &existing,
    };
    let result = writes
        .can_make_member_write(&caller(user_id, role), team_id, registered, write)
        .await;
    let seen = match result {
        Ok(ModelWriteGrant::Authorized) => "authorized".to_owned(),
        Ok(ModelWriteGrant::MemberRouter(grant)) => format!("member {grant}"),
        Err(MemberCallError::Access(error)) => {
            format!("{} {}", label(&error), rejection(&error).status.as_u16())
        }
        Err(MemberCallError::Refused(_)) => "refused".to_owned(),
    };
    assert_eq!(
        (seen.as_str(), policy.calls.lock().unwrap().len()),
        expected
    );
    assert!(org_roles.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn member_router_policy_sees_the_team_and_the_new_deployment() {
    let teams = FakeTeams::holding(&["team-1"]);
    let access = team_access(&Arc::new(CountedOrgRoles::default()));
    let writes = ModelWriteAccess {
        teams: &teams,
        team_access: &access,
    };
    let policy = RecordingPolicy {
        refuse: false,
        calls: Mutex::new(Vec::new()),
    };
    let deployment = "router".to_owned();
    let write = MemberWrite::Create {
        deployment: &deployment,
    };
    let grant = writes
        .can_make_member_write(
            &caller("member-1", ROLE),
            Some("team-1"),
            Some(&policy),
            write,
        )
        .await
        .unwrap();
    assert_eq!(
        grant,
        ModelWriteGrant::MemberRouter("create router".to_owned())
    );
    let calls = policy.calls.lock().unwrap().clone();
    assert_eq!(calls, vec!["team-1 member-1 create router".to_owned()]);
}

// LiteLLM answers these calls with 403 not_premium_user unless premium_user is true; betterlite has no paid tier
#[tokio::test]
async fn native_license_checks_and_paid_tier_quotas_are_absent_the_mit_behavior_is_ava() {
    let teams = FakeTeams::holding(&["team-1"]);
    let access = team_access(&Arc::new(CountedOrgRoles::default()));
    let writes = ModelWriteAccess {
        teams: &teams,
        team_access: &access,
    };
    let owner = caller("admin-1", ROLE);
    let stored = team("team-1");
    can_make_team_model_call("team-1", &owner, Some(&stored.fields.base)).unwrap();
    writes
        .can_make_model_call(&owner, Some("team-1"), ModelCall::TeamAction)
        .await
        .unwrap();
    writes
        .can_make_model_call(&owner, Some("team-1"), ModelCall::Write)
        .await
        .unwrap();
    assert_eq!(
        teams.reads(),
        vec!["team-1".to_owned(), "team-1".to_owned()]
    );
}

#[rstest]
#[case::view_only(ModelWriteError::ViewOnly, Some((403, "message")))]
#[case::member_router(ModelWriteError::MemberRouterUnavailable, Some((400, "message")))]
#[case::team_missing(ModelWriteError::TeamMissing { team_id: "gone".to_owned() }, Some((400, "error")))]
#[case::orphaned(ModelWriteError::OrphanedModel, Some((403, "error")))]
#[case::not_team_admin(ModelWriteError::NotTeamAdmin { team_id: "team-1".to_owned(), key_team: None }, Some((403, "error")))]
#[case::not_proxy_admin(ModelWriteError::NotProxyAdmin, Some((403, "error")))]
#[case::session_tags(ModelWriteError::AdminOnly { field: AdminOnlyField::SessionTags }, Some((403, "proxy")))]
#[case::team_read(ModelWriteError::TeamRead(Box::new(TeamQueryError::Record { source: TeamRecordError::NotAnObject })), None)]
#[case::team_access(ModelWriteError::TeamAccess(Box::new(TeamAccessError::Denied)), None)]
fn rejection_shapes(#[case] error: ModelWriteError, #[case] expected: Option<(u16, &str)>) {
    let shape = error.rejection().map(|found| {
        let kind = match found.detail {
            Detail::Message(_) => "message",
            Detail::Error(_) => "error",
            Detail::Proxy(_) => "proxy",
        };
        (found.status.as_u16(), kind)
    });
    assert_eq!(shape, expected);
}

#[derive(Debug, Clone, Copy)]
enum CredentialName {
    Absent,
    Null,
    Named(&'static str),
}

#[rstest]
#[case::absent("{}", CredentialName::Absent)]
#[case::null(r#"{"litellm_credential_name": null}"#, CredentialName::Null)]
#[case::named(
    r#"{"litellm_credential_name": "shared-openai"}"#,
    CredentialName::Named("shared-openai")
)]
fn model_patch_keeps_absent_apart_from_null(#[case] json: &str, #[case] expected: CredentialName) {
    let parsed: ModelPatch = serde_json::from_str(json).unwrap();
    let name = parsed
        .litellm_credential_name
        .as_ref()
        .map(Option::as_deref);
    let wanted = match expected {
        CredentialName::Absent => None,
        CredentialName::Null => Some(None),
        CredentialName::Named(stored_name) => Some(Some(stored_name)),
    };
    assert_eq!(name, wanted);
    let written = serde_json::to_string(&parsed).unwrap();
    let again: ModelPatch = serde_json::from_str(&written).unwrap();
    assert_eq!(again, parsed);
}

#[tokio::test]
async fn writer_team_rows_read_stored_teams() {
    postgres::writer_team_rows_read_stored_teams().await;
}

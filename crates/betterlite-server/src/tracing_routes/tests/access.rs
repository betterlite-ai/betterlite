use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_auth::auth_identity::roles::Role;
use betterlite_logging::trace_types::TraceScope;
use rstest::rstest;

use super::support::{principal, span_row};
use crate::auth::auth_key_lookup::tests::{caller_through_key_stage, master_caller};
use crate::tracing_routes::access::{
    AccessError, SystemClock, Tenant, TraceAuth, TraceClock as _, TraceRole, provide_trace_access,
};

#[rstest]
#[case::platform_admin(vec![Role::PlatformAdmin], TraceRole::Admin)]
#[case::platform_viewer(vec![Role::PlatformViewer], TraceRole::AdminViewOnly)]
#[case::team_member(vec![Role::TeamMember], TraceRole::Other)]
#[case::no_roles(vec![], TraceRole::Other)]
fn maps_roles(#[case] roles: Vec<Role>, #[case] expected: TraceRole) {
    let auth = TraceAuth::from_principal(&principal(roles, None, None, None, None));
    assert_eq!(auth.role, expected);
}

#[test]
fn a_key_whose_user_or_alias_is_master_key_is_not_an_admin() {
    let by_user = caller_through_key_stage(r#"{"user_id":"master_key"}"#);
    let by_alias = caller_through_key_stage(r#"{"key_alias":"master_key"}"#);
    let master = master_caller();
    assert_eq!(
        TraceAuth::from_principal(&by_user.principal).role,
        TraceRole::Other
    );
    assert_eq!(
        TraceAuth::from_principal(&by_alias.principal).role,
        TraceRole::Other
    );
    assert_eq!(
        TraceAuth::from_principal(&master.principal).role,
        TraceRole::Admin
    );
}

#[test]
fn reads_identity_fields_from_the_principal() {
    let caller = principal(
        vec![Role::TeamMember],
        Some("team-1"),
        Some("hash-1"),
        Some("org-1"),
        Some("project-1"),
    );
    let auth = TraceAuth::from_principal(&caller);
    assert_eq!(auth.team_id.as_deref(), Some("team-1"));
    assert_eq!(auth.token.as_deref(), Some("hash-1"));
    assert_eq!(auth.org_id.as_deref(), Some("org-1"));
    assert_eq!(auth.project_id.as_deref(), Some("project-1"));
}

#[test]
fn token_falls_back_to_the_key_id() {
    let mut caller = principal(vec![Role::TeamMember], None, None, None, None);
    caller.credential_ref.key_id = Some("key-hash".to_owned());
    let auth = TraceAuth::from_principal(&caller);
    assert_eq!(auth.token.as_deref(), Some("key-hash"));
}

#[test]
fn admin_reads_every_team_and_writes_with_its_identity() {
    let caller = principal(
        vec![Role::PlatformAdmin],
        Some("team-1"),
        Some("hash-1"),
        None,
        None,
    );
    let auth = TraceAuth::from_principal(&caller);
    let access = provide_trace_access(&auth, None);
    let scope = access.read_scope.expect("read scope");
    assert_eq!(
        scope,
        TraceScope {
            team_ids: Vec::new(),
            api_key_hash: String::new(),
        }
    );
    let tenant = access.write_tenant.expect("write tenant");
    assert_eq!(tenant.team_id, "team-1");
    assert_eq!(tenant.api_key_hash, "hash-1");
}

#[test]
fn admin_view_only_cannot_ingest() {
    let caller = principal(vec![Role::PlatformViewer], None, None, None, None);
    let auth = TraceAuth::from_principal(&caller);
    let access = provide_trace_access(&auth, None);
    assert!(access.read_scope.is_some());
    assert!(access.write_tenant.is_none());
}

#[test]
fn team_identity_scopes_reads_to_that_team() {
    let caller = principal(
        vec![Role::TeamMember],
        Some("team-1"),
        Some("hash-1"),
        None,
        None,
    );
    let auth = TraceAuth::from_principal(&caller);
    let access = provide_trace_access(&auth, None);
    assert_eq!(
        access.read_scope.expect("read scope"),
        TraceScope {
            team_ids: vec!["team-1".to_owned()],
            api_key_hash: String::new(),
        }
    );
    assert!(access.write_tenant.is_some());
}

#[test]
fn token_identity_scopes_reads_to_the_empty_team_and_the_token() {
    let caller = principal(vec![Role::TeamMember], None, Some("hash-1"), None, None);
    let auth = TraceAuth::from_principal(&caller);
    let access = provide_trace_access(&auth, None);
    assert_eq!(
        access.read_scope.expect("read scope"),
        TraceScope {
            team_ids: vec![String::new()],
            api_key_hash: "hash-1".to_owned(),
        }
    );
}

#[test]
fn a_caller_without_identity_cannot_read() {
    let caller = principal(vec![Role::TeamMember], None, None, None, None);
    let auth = TraceAuth::from_principal(&caller);
    let access = provide_trace_access(&auth, None);
    assert!(access.read_scope.is_none());
    assert!(access.write_tenant.is_some());
}

#[test]
fn reader_reports_unavailable_before_the_missing_scope() {
    let context = provide_trace_access(&TraceAuth::default(), None);
    let Err(error) = context.reader() else {
        panic!("a missing receiver must fail");
    };
    assert!(matches!(error, AccessError::Unavailable));
}

#[test]
fn writer_reports_the_missing_tenant_before_the_missing_receiver() {
    let context = provide_trace_access(
        &TraceAuth {
            role: TraceRole::AdminViewOnly,
            team_id: None,
            token: None,
            org_id: None,
            project_id: None,
        },
        None,
    );
    let Err(error) = context.writer() else {
        panic!("view only must not ingest");
    };
    assert!(matches!(error, AccessError::NotAllowedToIngest));
}

#[test]
fn tenant_stamps_rows_and_resource_attributes() {
    let tenant = Tenant {
        team_id: "team-1".to_owned(),
        api_key_hash: "hash-1".to_owned(),
        org_id: "org-1".to_owned(),
    };
    let mut rows = vec![span_row()];
    tenant.stamp_rows(&mut rows);
    let row = rows.first().expect("row");
    assert_eq!(row.team_id, "team-1");
    assert_eq!(row.api_key_hash, "hash-1");
    assert_eq!(
        row.resource_attributes.get("litellm.team_id"),
        Some(&"team-1".to_owned())
    );
    assert_eq!(
        row.resource_attributes.get("litellm.api_key_hash"),
        Some(&"hash-1".to_owned())
    );
    assert_eq!(
        row.resource_attributes.get("litellm.org_id"),
        Some(&"org-1".to_owned())
    );
    assert_eq!(
        row.resource_attributes.get("service.name"),
        Some(&"demo".to_owned())
    );
}

#[test]
fn the_system_clock_reads_unix_milliseconds() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_millis();
    let now = u128::try_from(SystemClock.now_ms()).expect("a positive reading");
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_millis();
    assert!(before <= now && now <= after);
}

#![cfg(test)]

mod adapters;
mod postgres;
mod support;

use std::sync::Arc;

use betterlite_auth::auth_identity::roles::Role;
use betterlite_auth::auth_user_lookup::cache::UserCache;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_db::model_secrets::DEFAULT_USER_ID;
use secrecy::SecretString;

use super::invite::team_admin_can_invite_user;
use super::ports::{TeamReadError, UserReadError};
use super::privileges::{admin_can_invite_user, user_has_admin_privileges};
use super::scope::AdminCaller;
use crate::auth::auth_key_lookup::master_key_context;
use support::{
    FakeCache, FakeTeams, FakeUsers, caller, capture_logs, invalid_team, org_user, principal,
    team_row, user, user_row,
};

#[tokio::test]
async fn the_master_key_is_a_proxy_admin_caller() {
    let master = SecretString::from("master".to_owned());
    let Ok(Some(context)) = master_key_context(Some(&master), &master) else {
        panic!("the master key authenticates");
    };
    let admin = AdminCaller::from_principal(&context.principal);
    assert!(admin.proxy_admin);
    assert_eq!(admin.user_id, Some(DEFAULT_USER_ID));
    let users = FakeUsers::default();
    assert!(user_has_admin_privileges(&admin, &users, None::<&FakeTeams>, None).await);
    assert!(users.calls().is_empty());
}

#[tokio::test]
async fn proxy_admin_before_missing_db() {
    let users = FakeUsers::default();
    let identity = principal(Some("u1"), vec![Role::PlatformAdmin]);
    let admin = AdminCaller::from_principal(&identity);
    assert!(user_has_admin_privileges(&admin, &users, None::<&FakeTeams>, None).await);
    assert!(admin_can_invite_user("absent", &admin, &users, None::<&FakeTeams>, None).await);
    assert!(users.calls().is_empty());
}

#[tokio::test]
async fn viewer_not_bypass() {
    let users = FakeUsers::new(vec![("u1", user("u1", &[]))]);
    let identity = principal(Some("u1"), vec![Role::PlatformViewer]);
    let viewer = AdminCaller::from_principal(&identity);
    let teams = FakeTeams::default();
    assert!(!user_has_admin_privileges(&viewer, &users, None::<&FakeTeams>, None).await);
    assert!(!user_has_admin_privileges(&viewer, &users, Some(&teams), None).await);
    assert_eq!(users.calls().len(), 1);
}

#[tokio::test]
async fn non_org_admin_membership_denies() {
    let users = FakeUsers::new(vec![(
        "u1",
        org_user("u1", &[("org-1", Some("internal_user")), ("org-2", None)]),
    )]);
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    assert!(!user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
}

#[tokio::test]
async fn no_user_no_db() {
    let users = FakeUsers::default();
    let anonymous = caller(None);
    assert!(!user_has_admin_privileges(&anonymous, &users, None::<&FakeTeams>, None).await);
    assert!(!admin_can_invite_user("u2", &anonymous, &users, None::<&FakeTeams>, None).await);
    assert!(users.calls().is_empty());
}

#[tokio::test]
async fn empty_user_id_lookup() {
    let users = FakeUsers::new(vec![("", org_user("", &[("org-1", Some("org_admin"))]))]);
    let teams = FakeTeams::default();
    let empty = caller(Some(""));
    assert!(user_has_admin_privileges(&empty, &users, Some(&teams), None).await);
    let calls = users.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls.first().map(|call| call.user_id.as_str()), Some(""));
}

#[tokio::test]
async fn user_upsert_disabled() {
    let users = FakeUsers::new(vec![
        ("u1", user("u1", &["t1"])),
        ("u2", user("u2", &["t1"])),
    ]);
    let teams = FakeTeams::new(vec![team_row("t1", &[("u1", "admin")])]);
    let admin = caller(Some("u1"));
    assert!(admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    let calls = users.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| !call.user_id_upsert));
}

#[tokio::test]
async fn fresh_cache_when_absent() {
    let users = FakeUsers::new(vec![("u1", user("u1", &[]))]);
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    assert!(!user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    assert!(!user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    let fresh = users.fresh();
    assert_eq!(fresh.len(), 2);
    let (Some(first), Some(second)) = (fresh.first(), fresh.get(1)) else {
        panic!("two calls build two fresh caches");
    };
    assert!(!Arc::ptr_eq(first, second));
    let calls = users.calls();
    let (Some(first_call), Some(second_call)) = (calls.first(), calls.get(1)) else {
        panic!("each call reads its user once");
    };
    assert!(Arc::ptr_eq(&first_call.cache, first));
    assert!(Arc::ptr_eq(&second_call.cache, second));
}

#[tokio::test]
async fn same_cache_two_reads() {
    let users = FakeUsers::new(vec![("u1", user("u1", &[])), ("u2", user("u2", &[]))]);
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    let supplied: Arc<dyn UserCache> = Arc::new(FakeCache::new(60.0_f64));
    let with_supplied = Some(Arc::clone(&supplied));
    assert!(!admin_can_invite_user("u2", &admin, &users, Some(&teams), with_supplied).await);
    let supplied_calls = users.calls();
    assert_eq!(supplied_calls.len(), 2);
    assert!(
        supplied_calls
            .iter()
            .all(|call| Arc::ptr_eq(&call.cache, &supplied))
    );
    assert!(users.fresh().is_empty());
    assert!(!admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    let calls = users.calls();
    let (Some(admin_read), Some(target_read)) = (calls.get(2), calls.get(3)) else {
        panic!("the second invite reads both users");
    };
    assert!(Arc::ptr_eq(&admin_read.cache, &target_read.cache));
    assert!(!Arc::ptr_eq(&admin_read.cache, &supplied));
}

#[tokio::test]
async fn admin_read_before_target() {
    let users = FakeUsers::new(vec![("u1", user("u1", &[])), ("u2", user("u2", &[]))]);
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    assert!(!admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    assert_eq!(
        users
            .calls()
            .iter()
            .map(|call| call.user_id.clone())
            .collect::<Vec<String>>(),
        vec!["u1".to_owned(), "u2".to_owned()]
    );
    let failing = FakeUsers::default();
    failing.fail(UserReadError::Lookup(UserLookupError::NoDatabase));
    assert!(!admin_can_invite_user("u2", &admin, &failing, Some(&teams), None).await);
    assert_eq!(failing.calls().len(), 1);
}

#[tokio::test]
async fn missing_user_denies() {
    let users = FakeUsers::default();
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    assert!(!user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    assert!(!admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    let present_admin = FakeUsers::new(vec![("u1", user("u1", &[]))]);
    assert!(!admin_can_invite_user("u2", &admin, &present_admin, Some(&teams), None).await);
    assert_eq!(present_admin.calls().len(), 2);
}

#[tokio::test]
async fn org_short_circuits_team_query() {
    let admin_row = user_row("u1", &["t1"], &[("org-1", Some("org_admin"))]);
    let target_row = user_row("u2", &["t1"], &[("org-1", Some("internal_user"))]);
    let users = FakeUsers::new(vec![("u1", admin_row), ("u2", target_row)]);
    let teams = FakeTeams::new(vec![team_row("t1", &[("u1", "admin")])]);
    let admin = caller(Some("u1"));
    assert!(user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    assert!(teams.requested().is_empty());
    assert!(admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    assert!(teams.requested().is_empty());
}

#[tokio::test]
async fn direct_team_error_propagates() {
    let admin_row = user("u1", &["t1"]);
    let target_row = user("u2", &["t1"]);
    let teams = FakeTeams::failing();
    let admin = caller(Some("u1"));
    assert!(matches!(
        team_admin_can_invite_user(&admin, &admin_row, &target_row, &teams).await,
        Err(TeamReadError::Database(_))
    ));
    let users = FakeUsers::new(vec![("u1", admin_row), ("u2", target_row)]);
    assert!(!admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
}

#[tokio::test]
async fn empty_teams_reject_before_query() {
    let teams = FakeTeams::failing();
    let admin = caller(Some("u1"));
    let empty_caller = user("u1", &[]);
    let target_row = user("u2", &["t1"]);
    assert!(matches!(
        team_admin_can_invite_user(&admin, &empty_caller, &target_row, &teams).await,
        Ok(false)
    ));
    let caller_row = user("u1", &["t1"]);
    let empty_target = user("u2", &[]);
    assert!(matches!(
        team_admin_can_invite_user(&admin, &caller_row, &empty_target, &teams).await,
        Ok(false)
    ));
    assert!(teams.requested().is_empty());
}

#[tokio::test]
async fn admin_lookup_error_debug_false() {
    let (buffer, _guard) = capture_logs(tracing::Level::DEBUG);
    let users = FakeUsers::default();
    users.fail(UserReadError::Lookup(UserLookupError::NoDatabase));
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    assert!(!user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    assert!(
        buffer
            .contents()
            .contains("Error checking admin privileges for user u1: No DB Connected")
    );
}

#[tokio::test]
async fn invite_lookup_error_debug_false() {
    let (buffer, _guard) = capture_logs(tracing::Level::DEBUG);
    let users = FakeUsers::default();
    users.fail(UserReadError::Lookup(UserLookupError::NoDatabase));
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    assert!(!admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    assert!(
        buffer
            .contents()
            .contains("Error checking invite permission for user u1: No DB Connected")
    );
}

#[tokio::test]
async fn invalid_team_row_denies() {
    let (buffer, _guard) = capture_logs(tracing::Level::DEBUG);
    let users = FakeUsers::new(vec![("u1", user("u1", &["t1"]))]);
    let teams = FakeTeams::new(vec![invalid_team()]);
    let admin = caller(Some("u1"));
    assert!(!user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    assert!(
        buffer
            .contents()
            .contains("Error checking admin privileges for user u1")
    );
    let admin_row = user("u1", &["t1"]);
    let target_row = user("u2", &["t1"]);
    assert!(matches!(
        team_admin_can_invite_user(&admin, &admin_row, &target_row, &teams).await,
        Err(TeamReadError::Record(_))
    ));
}

#[tokio::test]
async fn self_invite_needs_scope() {
    let users = FakeUsers::new(vec![("u1", user("u1", &["t1"]))]);
    let teams = FakeTeams::new(vec![team_row("t1", &[("u1", "user")])]);
    let admin = caller(Some("u1"));
    assert!(!admin_can_invite_user("u1", &admin, &users, Some(&teams), None).await);
    let led_teams = FakeTeams::new(vec![team_row("t1", &[("u1", "admin")])]);
    assert!(admin_can_invite_user("u1", &admin, &users, Some(&led_teams), None).await);
}

#[tokio::test]
async fn proxy_admin() {
    let users = FakeUsers::default();
    let identity = principal(None, vec![Role::PlatformAdmin]);
    let admin = AdminCaller::from_principal(&identity);
    let teams = FakeTeams::default();
    assert!(user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    assert!(admin_can_invite_user("", &admin, &users, Some(&teams), None).await);
    assert!(users.calls().is_empty());
}

#[tokio::test]
async fn shared_org() {
    let users = FakeUsers::new(vec![
        (
            "u1",
            org_user(
                "u1",
                &[
                    ("org-1", Some("org_admin")),
                    ("org-2", Some("internal_user")),
                ],
            ),
        ),
        ("u2", org_user("u2", &[("org-1", Some("internal_user"))])),
    ]);
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    assert!(admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    assert!(teams.requested().is_empty());
}

#[tokio::test]
async fn shared_team() {
    let users = FakeUsers::new(vec![
        ("u1", user("u1", &["t1", "t2"])),
        ("u2", user("u2", &["t2"])),
    ]);
    let teams = FakeTeams::new(vec![
        team_row("t1", &[("u1", "user")]),
        team_row("t2", &[("u1", "admin")]),
    ]);
    let admin = caller(Some("u1"));
    assert!(admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    assert_eq!(
        teams.requested(),
        vec![vec!["t1".to_owned(), "t2".to_owned()]]
    );
}

#[tokio::test]
async fn unrelated_target() {
    let admin_row = user_row("u1", &["t1"], &[("org-1", Some("org_admin"))]);
    let target_row = user_row("u2", &["t2"], &[("org-2", Some("internal_user"))]);
    let users = FakeUsers::new(vec![("u1", admin_row), ("u2", target_row)]);
    let teams = FakeTeams::new(vec![team_row("t1", &[("u1", "admin")])]);
    let admin = caller(Some("u1"));
    assert!(user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    assert!(!admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
}

#[tokio::test]
async fn lookup_failure_denies() {
    let users = FakeUsers::default();
    users.fail(UserReadError::Lookup(UserLookupError::NoDatabase));
    users.fail(UserReadError::Lookup(UserLookupError::NoDatabase));
    let teams = FakeTeams::default();
    let admin = caller(Some("u1"));
    assert!(!user_has_admin_privileges(&admin, &users, Some(&teams), None).await);
    assert!(!admin_can_invite_user("u2", &admin, &users, Some(&teams), None).await);
    assert_eq!(users.calls().len(), 2);
}

#[tokio::test]
async fn team_query_exact_ids() {
    postgres::team_query_exact_ids().await;
}

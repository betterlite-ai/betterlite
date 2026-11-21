#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::error::Error as _;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use betterlite_auth::auth_user_lookup::cache::UserCache;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::management_rbac::{FeatureSwitches, Settings};
use rstest::rstest;

use self::support::{
    CallerRecord, EmptyCache, FakePrivileges, FakeTeams, FakeUsers, LookupFailed, team_row,
    user_row,
};
use super::guard::check_feature_access;
use super::store::StorePrivileges;
use super::{AGENTS_DISABLED, FeatureAccessError, FeatureName, VECTOR_STORES_DISABLED};
use crate::management_admins::ports::TeamReads;

const ON: FeatureSwitches = FeatureSwitches {
    disabled: true,
    team_admins_allowed: true,
};
const DISABLED_ONLY: FeatureSwitches = FeatureSwitches {
    disabled: true,
    team_admins_allowed: false,
};
const ALLOWED_ONLY: FeatureSwitches = FeatureSwitches {
    disabled: false,
    team_admins_allowed: true,
};
const ALL_ON: Settings = Settings {
    agents: ON,
    vector_stores: ON,
};

const ORG_ADMIN_USER: &str = r#"{"user_id":"user-7","teams":[],"organization_memberships":[{"user_id":"user-7","organization_id":"org-1","user_role":"org_admin","created_at":"2026-01-01T00:00:00+00:00","updated_at":"2026-01-01T00:00:00+00:00"}]}"#;
const TEAM_MEMBER_USER: &str =
    r#"{"user_id":"user-7","teams":["team-1"],"organization_memberships":[]}"#;
const TEAM_ADMIN_ROSTER: &str =
    r#"{"team_id":"team-1","members_with_roles":[{"user_id":"user-7","role":"admin"}]}"#;
const TEAM_USER_ROSTER: &str =
    r#"{"team_id":"team-1","members_with_roles":[{"user_id":"user-7","role":"user"}]}"#;

fn answering(outcome: Result<bool, LookupFailed>) -> FakePrivileges {
    FakePrivileges {
        outcome,
        callers: Mutex::default(),
    }
}

fn callers(privileges: &FakePrivileges) -> Vec<CallerRecord> {
    privileges.callers.lock().unwrap().clone()
}

fn only(feature: FeatureName, switches: FeatureSwitches) -> Settings {
    let off = FeatureSwitches::default();
    match feature {
        FeatureName::Agents => Settings {
            agents: switches,
            vector_stores: off,
        },
        FeatureName::VectorStores => Settings {
            agents: off,
            vector_stores: switches,
        },
    }
}

fn teams(records: &[&str], unreachable: bool) -> FakeTeams {
    FakeTeams {
        records: records.iter().copied().map(team_row).collect(),
        unreachable,
        requested: Mutex::default(),
    }
}

fn users(json: Option<&str>) -> FakeUsers {
    FakeUsers {
        user: json.map(user_row),
        reads: Mutex::default(),
    }
}

#[rstest]
#[case::proxy_admin_agents(UserRole::ProxyAdmin, FeatureName::Agents)]
#[case::proxy_admin_vector_stores(UserRole::ProxyAdmin, FeatureName::VectorStores)]
#[case::viewer_agents(UserRole::ProxyAdminViewer, FeatureName::Agents)]
#[case::viewer_vector_stores(UserRole::ProxyAdminViewer, FeatureName::VectorStores)]
#[tokio::test]
async fn proxy_admin_unaffected(#[case] role: UserRole, #[case] feature: FeatureName) {
    let privileges = answering(Err(LookupFailed));
    check_feature_access(Some(role), Some("admin-1"), feature, &ALL_ON, &privileges)
        .await
        .unwrap();
    assert!(callers(&privileges).is_empty());
}

#[rstest]
#[case::agents(FeatureName::Agents, AGENTS_DISABLED)]
#[case::vector_stores(FeatureName::VectorStores, VECTOR_STORES_DISABLED)]
#[tokio::test]
async fn internal_feature_disabled(#[case] feature: FeatureName, #[case] message: &str) {
    let privileges = answering(Ok(true));
    let settings = only(feature, DISABLED_ONLY);
    let role = Some(UserRole::InternalUser);
    let error = check_feature_access(role, Some("user-7"), feature, &settings, &privileges)
        .await
        .unwrap_err();
    assert!(matches!(error, FeatureAccessError::Disabled(denied) if denied == feature));
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(error.to_string(), message);
    let detail = serde_json::to_string(&feature.denial()).unwrap();
    assert_eq!(detail, format!("{{\"error\":\"{message}\"}}"));
    assert!(callers(&privileges).is_empty());
}

#[test]
fn denial_details_name_the_feature() {
    assert_eq!(
        AGENTS_DISABLED,
        "Access to agents is disabled for your role. Contact your proxy admin."
    );
    assert_eq!(
        VECTOR_STORES_DISABLED,
        "Access to vector_stores is disabled for your role. Contact your proxy admin."
    );
}

#[rstest]
#[case::agents(FeatureName::Agents)]
#[case::vector_stores(FeatureName::VectorStores)]
#[tokio::test]
async fn team_admin_exemption(#[case] feature: FeatureName) {
    let privileges = answering(Ok(true));
    let role = Some(UserRole::InternalUser);
    check_feature_access(
        role,
        Some("user-7"),
        feature,
        &only(feature, ON),
        &privileges,
    )
    .await
    .unwrap();
    assert_eq!(callers(&privileges).len(), 1);
}

#[rstest]
#[case::org_admin(Some(UserRole::OrgAdmin))]
#[case::internal_user(Some(UserRole::InternalUser))]
#[case::internal_viewer(Some(UserRole::InternalUserViewer))]
#[case::team(Some(UserRole::Team))]
#[case::customer(Some(UserRole::Customer))]
#[case::no_role(None)]
#[tokio::test]
async fn disabled_blocks_every_other_role(#[case] role: Option<UserRole>) {
    let privileges = answering(Ok(true));
    let settings = only(FeatureName::Agents, DISABLED_ONLY);
    let refused = check_feature_access(
        role,
        Some("user-7"),
        FeatureName::Agents,
        &settings,
        &privileges,
    )
    .await;
    assert!(matches!(
        refused,
        Err(FeatureAccessError::Disabled(FeatureName::Agents))
    ));
}

#[rstest]
#[case::internal_user(Some(UserRole::InternalUser))]
#[case::org_admin(Some(UserRole::OrgAdmin))]
#[case::no_role(None)]
#[tokio::test]
async fn enabled_feature_passes_without_lookup(#[case] role: Option<UserRole>) {
    let privileges = answering(Err(LookupFailed));
    let settings = only(FeatureName::VectorStores, ALLOWED_ONLY);
    check_feature_access(
        role,
        None,
        FeatureName::VectorStores,
        &settings,
        &privileges,
    )
    .await
    .unwrap();
    assert!(callers(&privileges).is_empty());
}

#[rstest]
#[case::agents_off_vector_stores_asked(FeatureName::Agents, FeatureName::VectorStores)]
#[case::vector_stores_off_agents_asked(FeatureName::VectorStores, FeatureName::Agents)]
#[tokio::test]
async fn each_feature_reads_its_own_switches(
    #[case] disabled: FeatureName,
    #[case] asked: FeatureName,
) {
    let privileges = answering(Ok(false));
    let settings = only(disabled, ON);
    let role = Some(UserRole::InternalUser);
    check_feature_access(role, Some("user-7"), asked, &settings, &privileges)
        .await
        .unwrap();
    let refused =
        check_feature_access(role, Some("user-7"), disabled, &settings, &privileges).await;
    assert!(matches!(refused, Err(FeatureAccessError::Disabled(denied)) if denied == disabled));
}

#[rstest]
#[case::both_on(ON, 1)]
#[case::disabled_only(DISABLED_ONLY, 0)]
#[case::allowed_only(ALLOWED_ONLY, 0)]
#[case::both_off(FeatureSwitches::default(), 0)]
#[tokio::test]
async fn exemption_only_when_both_flags(#[case] switches: FeatureSwitches, #[case] calls: usize) {
    let privileges = answering(Ok(false));
    let settings = only(FeatureName::Agents, switches);
    let role = Some(UserRole::InternalUser);
    let outcome = check_feature_access(
        role,
        Some("user-7"),
        FeatureName::Agents,
        &settings,
        &privileges,
    )
    .await;
    assert_eq!(callers(&privileges).len(), calls);
    assert_eq!(outcome.is_err(), switches.disabled);
}

#[rstest]
#[case::org_admin(ORG_ADMIN_USER, &[])]
#[case::team_admin(TEAM_MEMBER_USER, &[TEAM_ADMIN_ROSTER])]
#[tokio::test]
async fn exemption_for_org_or_team_admin(#[case] user: &str, #[case] rosters: &[&str]) {
    let user_reads = users(Some(user));
    let team_reads = teams(rosters, false);
    let privileges = StorePrivileges {
        users: &user_reads,
        teams: Some(&team_reads),
        cache: Arc::new(EmptyCache),
    };
    let role = Some(UserRole::InternalUser);
    let settings = only(FeatureName::VectorStores, ON);
    check_feature_access(
        role,
        Some("user-7"),
        FeatureName::VectorStores,
        &settings,
        &privileges,
    )
    .await
    .unwrap();
}

#[rstest]
#[case::no_database(Some(TEAM_MEMBER_USER), &[TEAM_ADMIN_ROSTER], false, false)]
#[case::unknown_user(None, &[TEAM_ADMIN_ROSTER], true, false)]
#[case::member_not_admin(Some(TEAM_MEMBER_USER), &[TEAM_USER_ROSTER], true, false)]
#[case::team_read_fails(Some(TEAM_MEMBER_USER), &[TEAM_ADMIN_ROSTER], true, true)]
#[tokio::test]
async fn exemption_lookup_false_denies(
    #[case] user: Option<&str>,
    #[case] rosters: &[&str],
    #[case] connected: bool,
    #[case] unreachable: bool,
) {
    let user_reads = users(user);
    let team_reads = teams(rosters, unreachable);
    let database: &dyn TeamReads = &team_reads;
    let privileges = StorePrivileges {
        users: &user_reads,
        teams: connected.then_some(database),
        cache: Arc::new(EmptyCache),
    };
    let role = Some(UserRole::OrgAdmin);
    let settings = only(FeatureName::Agents, ON);
    let refused = check_feature_access(
        role,
        Some("user-7"),
        FeatureName::Agents,
        &settings,
        &privileges,
    )
    .await;
    assert!(matches!(
        refused,
        Err(FeatureAccessError::Disabled(FeatureName::Agents))
    ));
    assert_eq!(
        user_reads.reads.lock().unwrap().len(),
        usize::from(connected)
    );
}

#[tokio::test]
async fn exemption_false_from_the_port_denies() {
    let privileges = answering(Ok(false));
    let role = Some(UserRole::InternalUser);
    let settings = only(FeatureName::Agents, ON);
    let refused = check_feature_access(
        role,
        Some("user-7"),
        FeatureName::Agents,
        &settings,
        &privileges,
    )
    .await;
    assert!(matches!(
        refused,
        Err(FeatureAccessError::Disabled(FeatureName::Agents))
    ));
}

#[tokio::test]
async fn exemption_error_propagates() {
    let privileges = answering(Err(LookupFailed));
    let role = Some(UserRole::InternalUser);
    let settings = only(FeatureName::VectorStores, ON);
    let error = check_feature_access(
        role,
        Some("user-7"),
        FeatureName::VectorStores,
        &settings,
        &privileges,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        FeatureAccessError::Privileges(LookupFailed)
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.source().unwrap().to_string(),
        LookupFailed.to_string()
    );
}

#[rstest]
#[case::user_id(Some("user-7"), Some(UserRole::Team))]
#[case::no_user_id(None, None)]
#[tokio::test]
async fn exemption_call_arguments(#[case] user_id: Option<&str>, #[case] role: Option<UserRole>) {
    let privileges = answering(Ok(true));
    let settings = only(FeatureName::Agents, ON);
    check_feature_access(role, user_id, FeatureName::Agents, &settings, &privileges)
        .await
        .unwrap();
    assert_eq!(
        callers(&privileges),
        vec![(user_id.map(str::to_owned), false)]
    );
}

#[tokio::test]
async fn store_passes_the_caller_database_and_cache() {
    let user_reads = users(Some(TEAM_MEMBER_USER));
    let team_reads = teams(&[TEAM_USER_ROSTER], false);
    let cache: Arc<dyn UserCache> = Arc::new(EmptyCache);
    let privileges = StorePrivileges {
        users: &user_reads,
        teams: Some(&team_reads),
        cache: Arc::clone(&cache),
    };
    let role = Some(UserRole::InternalUser);
    let settings = only(FeatureName::Agents, ON);
    let refused = check_feature_access(
        role,
        Some("user-7"),
        FeatureName::Agents,
        &settings,
        &privileges,
    )
    .await;
    assert!(matches!(
        refused,
        Err(FeatureAccessError::Disabled(FeatureName::Agents))
    ));
    let reads = user_reads.reads.lock().unwrap().clone();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].0.as_deref(), Some("user-7"));
    assert!(Arc::ptr_eq(&reads[0].1, &cache));
    let requested = team_reads.requested.lock().unwrap().clone();
    assert_eq!(requested, vec![vec!["team-1".to_owned()]]);
}

#[rstest]
#[case::quoted_false("\"false\"", true)]
#[case::boolean_true("true", true)]
#[case::boolean_false("false", false)]
#[case::null_value("null", false)]
#[tokio::test]
async fn loaded_settings_reach_the_guard(#[case] value: &str, #[case] refused: bool) {
    let source =
        format!("general_settings:\n  disable_vector_stores_for_internal_users: {value}\n");
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let config = ConfigLoader { lookup }.load(&source).unwrap();
    let settings = config.sections.management_rbac;
    let privileges = answering(Ok(true));
    let role = Some(UserRole::InternalUser);
    let outcome = check_feature_access(
        role,
        Some("user-7"),
        FeatureName::VectorStores,
        &settings,
        &privileges,
    )
    .await;
    assert_eq!(outcome.is_err(), refused);
}

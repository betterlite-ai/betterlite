#![cfg(test)]

use rstest::rstest;

use super::{
    ADMIN_VIEWER_BASE_ROUTES, GLOBAL_SPEND_TRACKING_ROUTES, INTERNAL_USER_BASE_ROUTES,
    INTERNAL_USER_VIEW_ONLY_ROUTES, KEY_MANAGEMENT_ROUTES, MASTER_KEY_ONLY_ROUTES,
    ORG_ADMIN_ONLY_ROUTES, SPEND_TRACKING_ROUTES, TEAM_SERVICE_ACCOUNT_ROUTES, admin_viewer_routes,
    internal_user_routes, management_routes, org_admin_allowed_routes,
};

#[rstest]
#[case::key_generate(KEY_MANAGEMENT_ROUTES, 0, "/key/generate")]
#[case::admin_member_delete(ORG_ADMIN_ONLY_ROUTES, 4, "/organization/member_delete")]
fn route_literal_is_pinned(#[case] routes: &[&str], #[case] index: usize, #[case] literal: &str) {
    assert_eq!(routes.get(index), Some(&literal));
}

#[test]
fn whole_tables_match_litellm_shape() {
    assert_eq!(MASTER_KEY_ONLY_ROUTES.len(), 3);
    assert_eq!(KEY_MANAGEMENT_ROUTES.len(), 24);
    assert_eq!(TEAM_SERVICE_ACCOUNT_ROUTES.len(), 2);
    assert_eq!(super::MANAGEMENT_BASE_ROUTES.len(), 46);
    assert_eq!(SPEND_TRACKING_ROUTES.len(), 16);
    assert_eq!(GLOBAL_SPEND_TRACKING_ROUTES.len(), 12);
    assert_eq!(INTERNAL_USER_BASE_ROUTES.len(), 18);
    assert_eq!(INTERNAL_USER_VIEW_ONLY_ROUTES.len(), 25);
    assert_eq!(ORG_ADMIN_ONLY_ROUTES.len(), 5);
    assert_eq!(ADMIN_VIEWER_BASE_ROUTES.len(), 53);
}

#[test]
fn group_expansion_keeps_order_and_duplicates() {
    assert_eq!(management_routes().len(), 46 + 24 + 3 + 5);
    assert_eq!(internal_user_routes().len(), 18 + 16 + 24 + 2);
    assert_eq!(
        admin_viewer_routes().len(),
        53 + 16 + super::INFO_ROUTES.len()
    );
    assert_eq!(
        org_admin_allowed_routes().len(),
        5 + management_routes().len() + 69 + admin_viewer_routes().len()
    );
    assert_eq!(
        management_routes().last(),
        super::AGENT_MANAGEMENT_ROUTES.last(),
    );
}

#[test]
fn internal_roles_exclude_global_spend() {
    assert!(
        internal_user_routes()
            .iter()
            .all(|route| !GLOBAL_SPEND_TRACKING_ROUTES.contains(route))
    );
}

#[test]
fn key_health_is_not_key_management() {
    assert!(
        !KEY_MANAGEMENT_ROUTES.contains(&super::key_routes::KeyManagementRoute::KeyHealth.value(),)
    );
}

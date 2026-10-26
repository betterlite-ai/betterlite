#![cfg(test)]

#[path = "tests/admin_viewer/tests.rs"]
mod admin_viewer;
#[path = "tests/global_spend_tracking/tests.rs"]
mod global_spend_tracking;
#[path = "tests/internal_user/tests.rs"]
mod internal_user;
#[path = "tests/internal_user_view_only/tests.rs"]
mod internal_user_view_only;
#[path = "tests/key_management/tests.rs"]
mod key_management;
#[path = "tests/management/tests.rs"]
mod management;
#[path = "tests/master_key_only/tests.rs"]
mod master_key_only;
#[path = "tests/org_admin_only/tests.rs"]
mod org_admin_only;
#[path = "tests/service_account/tests.rs"]
mod service_account;
#[path = "tests/spend_tracking/tests.rs"]
mod spend_tracking;

#[path = "tests/key_enum/tests.rs"]
mod key_enum;

use super::{
    ADMIN_VIEWER_BASE_ROUTES, GLOBAL_SPEND_TRACKING_ROUTES, INTERNAL_USER_BASE_ROUTES,
    INTERNAL_USER_VIEW_ONLY_ROUTES, KEY_MANAGEMENT_ROUTES, MANAGEMENT_BASE_ROUTES,
    ORG_ADMIN_ONLY_ROUTES, SPEND_TRACKING_ROUTES, admin_viewer_routes, internal_user_routes,
    management_routes, org_admin_allowed_routes,
};

#[test]
fn group_expansion() {
    assert_eq!(
        management_routes(),
        [
            MANAGEMENT_BASE_ROUTES,
            KEY_MANAGEMENT_ROUTES,
            super::MCP_MANAGEMENT_ROUTES,
            super::AGENT_MANAGEMENT_ROUTES
        ]
        .concat()
    );
    assert_eq!(
        internal_user_routes(),
        [
            INTERNAL_USER_BASE_ROUTES,
            SPEND_TRACKING_ROUTES,
            KEY_MANAGEMENT_ROUTES,
            super::COMPLIANCE_CHECK_ROUTES
        ]
        .concat()
    );
    assert_eq!(
        admin_viewer_routes(),
        [
            ADMIN_VIEWER_BASE_ROUTES,
            SPEND_TRACKING_ROUTES,
            super::INFO_ROUTES
        ]
        .concat()
    );
    assert_eq!(
        org_admin_allowed_routes(),
        [
            ORG_ADMIN_ONLY_ROUTES,
            &management_routes(),
            super::SELF_MANAGED_ROUTES,
            &admin_viewer_routes()
        ]
        .concat()
    );
}

#[test]
fn internal_viewer_expansion() {
    let tag_routes = [
        "/tag/daily/activity",
        "/tag/daily/activity/aggregated",
        "/tag/daily/activity/aggregated/keys",
        "/tag/daily/activity/aggregated/search",
        "/tag/daily/activity/aggregated/model_top_keys",
        "/tag/daily/activity/export",
        "/tag/list",
    ];
    assert_eq!(
        INTERNAL_USER_VIEW_ONLY_ROUTES,
        [
            SPEND_TRACKING_ROUTES,
            super::COMPLIANCE_CHECK_ROUTES,
            &tag_routes
        ]
        .concat()
    );
    assert!(!INTERNAL_USER_VIEW_ONLY_ROUTES.contains(&"/key/generate"));
    assert!(!INTERNAL_USER_VIEW_ONLY_ROUTES.contains(&"/key/update"));
}

#[test]
fn internal_roles_exclude_global_spend() {
    assert!(
        internal_user_routes()
            .iter()
            .all(|route| !GLOBAL_SPEND_TRACKING_ROUTES.contains(route))
    );
    assert!(
        INTERNAL_USER_VIEW_ONLY_ROUTES
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

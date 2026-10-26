pub mod key_routes;
mod tests;

use crate::auxiliary_routes::{
    AGENT_MANAGEMENT_ROUTES, COMPLIANCE_CHECK_ROUTES, INFO_ROUTES, MCP_MANAGEMENT_ROUTES,
    SELF_MANAGED_ROUTES,
};

pub const MASTER_KEY_ONLY_ROUTES: &[&str] = &[
    "/global/spend/reset",
    "/memory-usage-in-mem-cache",
    "/memory-usage-in-mem-cache-items",
];

pub const KEY_MANAGEMENT_ROUTES: &[&str] = &[
    "/key/generate",
    "/key/update",
    "/key/delete",
    "/key/info",
    "/key/regenerate",
    "/key/service-account/generate",
    "/key/{key_id}/regenerate",
    "/key/list",
    "/key/block",
    "/key/unblock",
    "/key/bulk_update",
    "/team/key/bulk_update",
    "/team/daily/activity",
    "/team/daily/activity/aggregated",
    "/team/daily/activity/aggregated/keys",
    "/team/daily/activity/aggregated/search",
    "/team/daily/activity/aggregated/model_top_keys",
    "/team/daily/activity/export",
    "/spend/logs",
    "/spend/logs/v2",
    "/key/{key_id}/reset_spend",
    "/key/aliases",
    "/key/access_group_assignment",
    "/auto_router/manage",
];

pub const TEAM_SERVICE_ACCOUNT_ROUTES: &[&str] = &["/key/generate", "/key/update"];

pub const MANAGEMENT_BASE_ROUTES: &[&str] = &[
    "/user/new",
    "/management/v1/users/bulk",
    "/user/update",
    "/user/bulk_update",
    "/user/delete",
    "/management/v1/users/bulk_delete",
    "/user/info",
    "/user/list",
    "/user/daily/activity",
    "/user/daily/activity/aggregated",
    "/user/daily/activity/aggregated/keys",
    "/user/daily/activity/aggregated/search",
    "/user/daily/activity/aggregated/model_top_keys",
    "/user/daily/activity/export",
    "/user/daily/activity/aggregated/cache_leakage_keys",
    "/team/new",
    "/team/update",
    "/team/{team_id}",
    "/team/delete",
    "/team/list",
    "/v2/team/list",
    "/team/info",
    "/team/block",
    "/team/unblock",
    "/team/available",
    "/team/metadata_schema",
    "/team/permissions_list",
    "/team/permissions_update",
    "/team/permissions_bulk_update",
    "/team/daily/activity",
    "/team/daily/activity/aggregated",
    "/team/daily/activity/aggregated/keys",
    "/team/daily/activity/aggregated/search",
    "/team/daily/activity/aggregated/model_top_keys",
    "/team/daily/activity/export",
    "/team/spend/by_user",
    "/gateway/daily/activity",
    "/model/new",
    "/model/update",
    "/model/delete",
    "/model/info",
    "/jwt/key/mapping/new",
    "/jwt/key/mapping/update",
    "/jwt/key/mapping/delete",
    "/jwt/key/mapping/list",
    "/jwt/key/mapping/info",
];

pub const SPEND_TRACKING_ROUTES: &[&str] = &[
    "/spend/keys",
    "/spend/users",
    "/spend/tags",
    "/spend/calculate",
    "/spend/logs",
    "/spend/logs/v2",
    "/spend/logs/ui",
    "/spend/logs/ui/{request_id}",
    "/spend/logs/session/ui",
    "/key/spend/report",
    "/user/spend/report",
    "/team/spend/report",
    "/organization/spend/report",
    "/management/v1/spend_logs/end_users",
    "/management/v1/spend_logs/users",
    "/cost/estimate",
];

pub const GLOBAL_SPEND_TRACKING_ROUTES: &[&str] = &[
    "/global/spend/logs",
    "/global/spend",
    "/global/spend/keys",
    "/global/spend/teams",
    "/global/spend/end_users",
    "/global/spend/models",
    "/global/predict/spend/logs",
    "/global/spend/report",
    "/global/spend/provider",
    "/global/spend/tags",
    "/global/spend/all_tag_names",
    "/spend/capture_rate",
];

pub const INTERNAL_USER_BASE_ROUTES: &[&str] = &[
    "/global/activity",
    "/global/activity/model",
    "/global/activity/cache_hits",
    "/tag/daily/activity",
    "/tag/daily/activity/aggregated",
    "/tag/daily/activity/aggregated/keys",
    "/tag/daily/activity/aggregated/search",
    "/tag/daily/activity/aggregated/model_top_keys",
    "/tag/daily/activity/export",
    "/tag/list",
    "/v1/models/{model_id}",
    "/models/{model_id}",
    "/guardrails/list",
    "/v2/guardrails/list",
    "/project/list",
    "/project/info",
    "/search_tools/list",
    "/search_tools/ui/available_providers",
];

pub const INTERNAL_USER_VIEW_ONLY_ROUTES: &[&str] = &[
    "/spend/keys",
    "/spend/users",
    "/spend/tags",
    "/spend/calculate",
    "/spend/logs",
    "/spend/logs/v2",
    "/spend/logs/ui",
    "/spend/logs/ui/{request_id}",
    "/spend/logs/session/ui",
    "/key/spend/report",
    "/user/spend/report",
    "/team/spend/report",
    "/organization/spend/report",
    "/management/v1/spend_logs/end_users",
    "/management/v1/spend_logs/users",
    "/cost/estimate",
    "/compliance/eu-ai-act",
    "/compliance/gdpr",
    "/tag/daily/activity",
    "/tag/daily/activity/aggregated",
    "/tag/daily/activity/aggregated/keys",
    "/tag/daily/activity/aggregated/search",
    "/tag/daily/activity/aggregated/model_top_keys",
    "/tag/daily/activity/export",
    "/tag/list",
];

pub const ORG_ADMIN_ONLY_ROUTES: &[&str] = &[
    "/organization/info",
    "/organization/delete",
    "/organization/member_add",
    "/organization/member_update",
    "/organization/member_delete",
];

pub const ADMIN_VIEWER_BASE_ROUTES: &[&str] = &[
    "/user/list",
    "/user/available_users",
    "/user/available_roles",
    "/user/daily/activity",
    "/user/daily/activity/aggregated",
    "/user/daily/activity/aggregated/keys",
    "/user/daily/activity/aggregated/search",
    "/user/daily/activity/aggregated/model_top_keys",
    "/user/daily/activity/export",
    "/user/daily/activity/aggregated/cache_leakage_keys",
    "/team/daily/activity",
    "/team/daily/activity/aggregated",
    "/team/daily/activity/aggregated/keys",
    "/team/daily/activity/aggregated/search",
    "/team/daily/activity/aggregated/model_top_keys",
    "/team/daily/activity/export",
    "/tag/daily/activity",
    "/tag/daily/activity/aggregated",
    "/tag/daily/activity/aggregated/keys",
    "/tag/daily/activity/aggregated/search",
    "/tag/daily/activity/aggregated/model_top_keys",
    "/tag/daily/activity/export",
    "/tag/list",
    "/audit",
    "/audit/{id}",
    "/global/activity",
    "/global/activity/model",
    "/global/activity/cache_hits",
    "/customer/list",
    "/customer/info",
    "/spend/logs/session/ui",
    "/management/v1/spend_logs/end_users",
    "/management/v1/spend_logs/users",
    "/callbacks/list",
    "/callbacks/configs",
    "/get/config/callbacks",
    "/alerting/settings",
    "/config/list",
    "/config/field/info",
    "/budget/list",
    "/management/v1/budgets",
    "/budget/settings",
    "/invitation/info",
    "/guardrails/list",
    "/v2/guardrails/list",
    "/guardrails/submissions",
    "/guardrails/submissions/{guardrail_id}",
    "/guardrails/usage/overview",
    "/policies/attachments/list",
    "/get/mcp_semantic_filter_settings",
    "/schedule/model_cost_map_reload/status",
    "/model/cost_map/source",
    "/auto_router/classifier/default_prompt",
];

#[must_use]
pub fn management_routes() -> Vec<&'static str> {
    let mut routes = MANAGEMENT_BASE_ROUTES.to_vec();
    routes.extend_from_slice(KEY_MANAGEMENT_ROUTES);
    routes.extend_from_slice(MCP_MANAGEMENT_ROUTES);
    routes.extend_from_slice(AGENT_MANAGEMENT_ROUTES);
    routes
}

#[must_use]
pub fn internal_user_routes() -> Vec<&'static str> {
    let mut routes = INTERNAL_USER_BASE_ROUTES.to_vec();
    routes.extend_from_slice(SPEND_TRACKING_ROUTES);
    routes.extend_from_slice(KEY_MANAGEMENT_ROUTES);
    routes.extend_from_slice(COMPLIANCE_CHECK_ROUTES);
    routes
}

#[must_use]
pub fn admin_viewer_routes() -> Vec<&'static str> {
    let mut routes = ADMIN_VIEWER_BASE_ROUTES.to_vec();
    routes.extend_from_slice(SPEND_TRACKING_ROUTES);
    routes.extend_from_slice(INFO_ROUTES);
    routes
}

#[must_use]
pub fn org_admin_allowed_routes() -> Vec<&'static str> {
    let mut routes = ORG_ADMIN_ONLY_ROUTES.to_vec();
    routes.extend_from_slice(&management_routes());
    routes.extend_from_slice(SELF_MANAGED_ROUTES);
    routes.extend_from_slice(&admin_viewer_routes());
    routes
}

#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::management_routes::management_routes;
use crate::pass_through_registry::{
    PassThroughMetadata, PassThroughRegistry, PassThroughRouteType,
};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use crate::route_match::templates::check_route_access;
use crate::user_roles::UserRole;

use super::admission::{RoutePolicyInput, is_api_route_allowed};
use super::jwt::allowed_routes_check;
use super::mask::mask_user_id;
use super::template::template_matches;
use super::viewer::check_access;
use super::viewer_tables::{
    EARLY_BLOCKED_KEY_SUFFIXES, EARLY_BLOCKED_ROUTES, LATE_BLOCKED_KEY_SUFFIX,
    LATE_BLOCKED_WRITE_ROUTES,
};

const ADMIN_ROW: &str = r#"{"user_id":"admin-1","user_role":"proxy_admin"}"#;
const INTERNAL_ROW: &str = r#"{"user_id":"user-1","user_role":"internal_user"}"#;
const VIEWER_ROW: &str = r#"{"user_id":"viewer-1","user_role":"proxy_admin_viewer"}"#;
const WATCHER_ROW: &str = r#"{"user_id":"watcher-1","user_role":"internal_user_viewer"}"#;
const CUSTOMER_ROW: &str = r#"{"user_id":"buyer-1","user_role":"customer"}"#;
const ORG_ADMIN_ROW: &str = r#"{"user_id":"user-1","user_role":"internal_user","organization_memberships":[{"user_id":"user-1","organization_id":"org-a","user_role":"org_admin","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}]}"#;
const INTERNAL_TOKEN: &str = r#"{"user_id":"user-1"}"#;
const VIEWER_TOKEN: &str = r#"{"user_id":"viewer-1"}"#;
const WATCHER_TOKEN: &str = r#"{"user_id":"watcher-1"}"#;
const BUYER_TOKEN: &str = r#"{"user_id":"buyer-1"}"#;
const ADMIN_DEFAULT_ROUTES: [&str; 4] = [
    "management_routes",
    "spend_tracking_routes",
    "global_spend_tracking_routes",
    "info_routes",
];
const TEAM_DEFAULT_ROUTES: [&str; 5] = [
    "openai_routes",
    "info_routes",
    "mcp_routes",
    "/v1/messages",
    "/v1/messages/count_tokens",
];

fn body(fields: &str) -> JsonObject {
    serde_json::from_str(fields).unwrap()
}

fn key(fields: &str) -> UserAPIKeyAuth {
    let record: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(record)).unwrap()
}

fn user(fields: &str) -> LiteLLMUserTable {
    let record: JsonData = serde_json::from_str(fields).unwrap();
    user_record_to_model(Some(&record)).unwrap().unwrap()
}

fn owned(routes: &[&str]) -> Vec<String> {
    routes.iter().map(|route| (*route).to_owned()).collect()
}

struct Policy {
    route: String,
    method: Option<String>,
    user_id_query: Option<String>,
    body: JsonObject,
    token: Option<UserAPIKeyAuth>,
    user: Option<LiteLLMUserTable>,
    registry: PassThroughRegistry,
    admin_only_routes: Vec<String>,
}

impl Policy {
    fn new(route: &str) -> Self {
        Self {
            route: route.to_owned(),
            method: None,
            user_id_query: None,
            body: JsonObject::new(),
            token: None,
            user: None,
            registry: PassThroughRegistry {
                root_path: String::new(),
                registrations: Vec::new(),
            },
            admin_only_routes: Vec::new(),
        }
    }

    #[must_use]
    fn with_token(mut self, fields: &str) -> Self {
        self.token = Some(key(fields));
        self
    }

    #[must_use]
    fn with_user(mut self, fields: &str) -> Self {
        self.user = Some(user(fields));
        self
    }

    #[must_use]
    fn with_method(mut self, method: &str) -> Self {
        self.method = Some(method.to_owned());
        self
    }

    #[must_use]
    fn with_query(mut self, user_id: &str) -> Self {
        self.user_id_query = Some(user_id.to_owned());
        self
    }

    #[must_use]
    fn with_body(mut self, fields: &str) -> Self {
        self.body = body(fields);
        self
    }

    #[must_use]
    fn with_admin_only(mut self, routes: &[&str]) -> Self {
        self.admin_only_routes = owned(routes);
        self
    }

    #[must_use]
    fn with_registration(mut self, path: &str, auth: bool) -> Self {
        self.registry.registrations.push((
            format!("policy-endpoint:exact:{path}:POST"),
            PassThroughMetadata {
                endpoint_id: "policy-endpoint".to_owned(),
                route_type: PassThroughRouteType::Exact,
                path: path.to_owned(),
                methods: vec!["POST".to_owned()],
                auth: Some(auth),
            },
        ));
        self
    }

    fn run(&self) -> Result<(), Box<ProxyException>> {
        is_api_route_allowed(&RoutePolicyInput {
            route: &self.route,
            method: self.method.as_deref(),
            user_id_query: self.user_id_query.as_deref(),
            body: &self.body,
            token: self.token.as_ref(),
            user: self.user.as_ref(),
            registry: &self.registry,
            admin_only_routes: &self.admin_only_routes,
            team_allowed_routes: &[],
        })
    }
}

fn assert_auth_error(result: Result<(), Box<ProxyException>>, status: &str) {
    let exception = result.expect_err("the route policy denies the caller");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, status);
}

fn viewer(route: &str, method: Option<&str>) -> Result<(), Box<ProxyException>> {
    check_access(route, method, &JsonObject::new())
}

fn viewer_body(route: &str, method: &str, fields: &str) -> Result<(), Box<ProxyException>> {
    check_access(route, Some(method), &body(fields))
}

#[test]
fn null_token_admin_fails() {
    let policy = Policy::new("/user/new").with_user(ADMIN_ROW);
    let exception = policy
        .run()
        .expect_err("a live proxy admin without a token is refused");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "401");
    assert_eq!(
        exception.message,
        "Invalid proxy server token passed. valid_token=None."
    );
}

#[test]
fn live_admin_bypasses_policy() {
    let admin = Policy::new("/user/new")
        .with_user(ADMIN_ROW)
        .with_token(INTERNAL_TOKEN)
        .with_admin_only(&["/user/new"]);
    admin.run().unwrap();
    let internal = Policy::new("/user/new")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN)
        .with_admin_only(&["/user/new"]);
    assert_auth_error(internal.run(), "403");
}

// LiteLLM returns before the admin_only_routes check unless premium_user is true; betterlite enforces the list
#[test]
fn native_configured_admin_only_routes_is_enforced_without_premium_user() {
    let listed = Policy::new("/user/new")
        .with_method("POST")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN)
        .with_admin_only(&["/user/new"]);
    let exception = listed
        .run()
        .expect_err("a configured admin-only route refuses an internal user");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "403");
    let admin = Policy::new("/user/new")
        .with_method("POST")
        .with_user(ADMIN_ROW)
        .with_token(INTERNAL_TOKEN)
        .with_admin_only(&["/user/new"]);
    admin.run().unwrap();
}

#[test]
fn custom_admin_check_first() {
    let listed = Policy::new("/v1/chat/completions")
        .with_method("POST")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN)
        .with_admin_only(&["/v1/chat/completions"]);
    assert_auth_error(listed.run(), "403");
    let unlisted = Policy::new("/v1/chat/completions")
        .with_method("POST")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN)
        .with_admin_only(&["/user/new"]);
    unlisted.run().unwrap();
}

#[test]
fn pass_through_before_llm() {
    let denied = Policy::new("/v1/embeddings")
        .with_method("POST")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN)
        .with_registration("/v1/embeddings", true);
    assert_auth_error(denied.run(), "403");
    let granted = Policy::new("/v1/embeddings")
        .with_method("POST")
        .with_user(INTERNAL_ROW)
        .with_token(
            r#"{"user_id":"user-1","metadata":{"allowed_passthrough_routes":["/v1/embeddings"]}}"#,
        )
        .with_registration("/v1/embeddings", true);
    granted.run().unwrap();
}

#[test]
fn llm_before_viewer_check() {
    let policy = Policy::new("/v1/chat/completions")
        .with_method("POST")
        .with_user(VIEWER_ROW)
        .with_token(VIEWER_TOKEN);
    policy.run().unwrap();
}

#[rstest]
#[case::key_info("/key/info")]
#[case::v2_user_info("/v2/user/info")]
#[case::model_info("/model/info")]
#[case::team_info("/team/info")]
fn info_handler_deferral(#[case] route: &str) {
    let policy = Policy::new(route)
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN);
    policy.run().unwrap();
}

#[test]
fn other_user_info_403() {
    let policy = Policy::new("/user/info")
        .with_method("GET")
        .with_query("user-2")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN);
    assert_auth_error(policy.run(), "403");
}

#[rstest]
#[case::empty("")]
#[case::own("user-1")]
fn empty_user_id_passes(#[case] query: &str) {
    let policy = Policy::new("/user/info")
        .with_method("GET")
        .with_query(query)
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN);
    policy.run().unwrap();
    let absent = Policy::new("/user/info")
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN);
    absent.run().unwrap();
}

#[test]
fn viewer_other_user_info_passes() {
    let policy = Policy::new("/user/info")
        .with_method("GET")
        .with_query("user-2")
        .with_user(VIEWER_ROW)
        .with_token(VIEWER_TOKEN);
    policy.run().unwrap();
}

#[test]
fn spend_permission_required() {
    let without = Policy::new("/global/spend/logs")
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN);
    assert_auth_error(without.run(), "401");
    let granted = Policy::new("/global/spend/logs")
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(r#"{"user_id":"user-1","permissions":{"get_spend_routes":true}}"#);
    granted.run().unwrap();
}

#[rstest]
#[case::search_tools("/search_tools/list")]
#[case::search_tools_ui("/search_tools/ui/available_providers")]
fn internal_user_routes(#[case] route: &str) {
    let policy = Policy::new(route)
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN);
    policy.run().unwrap();
    let customer = Policy::new("/search_tools/list")
        .with_method("GET")
        .with_user(CUSTOMER_ROW)
        .with_token(BUYER_TOKEN);
    assert_auth_error(customer.run(), "401");
    let elsewhere = Policy::new("/organization/info")
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(INTERNAL_TOKEN);
    assert_auth_error(elsewhere.run(), "401");
}

#[test]
fn org_admin_body_scope() {
    let own = Policy::new("/organization/info")
        .with_method("POST")
        .with_body(r#"{"organization_id":"org-a"}"#)
        .with_user(ORG_ADMIN_ROW)
        .with_token(INTERNAL_TOKEN);
    own.run().unwrap();
    let foreign = Policy::new("/organization/info")
        .with_method("POST")
        .with_body(r#"{"organization_id":"org-b"}"#)
        .with_user(ORG_ADMIN_ROW)
        .with_token(INTERNAL_TOKEN);
    assert_auth_error(foreign.run(), "401");
    let absent = Policy::new("/organization/info")
        .with_method("POST")
        .with_user(ORG_ADMIN_ROW)
        .with_token(INTERNAL_TOKEN);
    assert_auth_error(absent.run(), "401");
}

#[rstest]
#[case::spend_keys("/spend/keys")]
#[case::compliance_gdpr("/compliance/gdpr")]
fn internal_viewer_routes(#[case] route: &str) {
    Policy::new(route)
        .with_method("GET")
        .with_user(WATCHER_ROW)
        .with_token(WATCHER_TOKEN)
        .run()
        .unwrap();
    assert_auth_error(
        Policy::new("/user/new")
            .with_method("POST")
            .with_user(WATCHER_ROW)
            .with_token(WATCHER_TOKEN)
            .run(),
        "401",
    );
    assert_auth_error(
        Policy::new("/spend/keys")
            .with_method("GET")
            .with_user(CUSTOMER_ROW)
            .with_token(BUYER_TOKEN)
            .run(),
        "401",
    );
}

#[test]
fn self_managed_routes() {
    let policy = Policy::new("/team/update")
        .with_method("POST")
        .with_user(CUSTOMER_ROW)
        .with_token(BUYER_TOKEN);
    policy.run().unwrap();
}

#[rstest]
#[case::mcp_server("/v1/mcp/server")]
#[case::mcp_rest("/mcp-rest/tools")]
fn mcp_prefix_deferral(#[case] route: &str) {
    let policy = Policy::new(route)
        .with_method("POST")
        .with_user(CUSTOMER_ROW)
        .with_token(BUYER_TOKEN);
    policy.run().unwrap();
}

#[test]
fn team_service_account_routes() {
    let service = Policy::new("/key/generate")
        .with_method("POST")
        .with_token(r#"{"team_id":"team-1","metadata":{"service_account_id":"sa-1"}}"#);
    service.run().unwrap();
    let user_owned = Policy::new("/key/generate").with_method("POST").with_token(
        r#"{"user_id":"service-1","team_id":"team-1","metadata":{"service_account_id":"sa-1"}}"#,
    );
    assert_auth_error(user_owned.run(), "401");
}

#[rstest]
#[case::exact("/internal-models", true)]
#[case::child("/internal-models/one", true)]
#[case::wildcard("/scim/v2/Users", true)]
#[case::exact_route("/exact/route", true)]
#[case::sibling("/internal-models-extra", false)]
#[case::other_version("/scim/v3/Users", false)]
#[case::other_exact("/exact/other", false)]
fn explicit_allowed_route_match(#[case] route: &str, #[case] allowed: bool) {
    let token =
        r#"{"user_id":"user-1","allowed_routes":["/internal-models","/scim/v2/*","/exact/route"]}"#;
    let policy = Policy::new(route)
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(token);
    if allowed {
        policy.run().unwrap();
    } else {
        assert_auth_error(policy.run(), "401");
    }
}

#[test]
fn empty_allowed_routes_401() {
    let policy = Policy::new("/internal-models/one")
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(r#"{"user_id":"user-1","allowed_routes":[]}"#);
    assert_auth_error(policy.run(), "401");
}

#[rstest]
#[case::absent(INTERNAL_TOKEN)]
#[case::null(r#"{"user_id":"user-1","allowed_routes":null}"#)]
fn absent_allowed_routes_401(#[case] token: &str) {
    let policy = Policy::new("/internal-models/one")
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(token);
    assert_auth_error(policy.run(), "401");
}

#[test]
fn non_text_allowed_routes_entry_is_skipped() {
    let mixed = Policy::new("/internal-models/one")
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(r#"{"user_id":"user-1","allowed_routes":[7,"/internal-models"]}"#);
    mixed.run().unwrap();
    let number_only = Policy::new("/internal-models/one")
        .with_method("GET")
        .with_user(INTERNAL_ROW)
        .with_token(r#"{"user_id":"user-1","allowed_routes":[7]}"#);
    assert_auth_error(number_only.run(), "401");
}

#[test]
fn denied_message_masks_the_caller() {
    let named = Policy::new("/nothing/here")
        .with_user(r#"{"user_id":"alice-long-id","user_role":""}"#)
        .with_token(r#"{"user_id":"alice-long-id"}"#);
    let exception = named
        .run()
        .expect_err("the fallback denies an unknown route");
    assert!(exception.message.contains("Your role=unknown"));
    assert!(exception.message.contains("Your user_id=alice-*****id"));
    let anonymous = Policy::new("/nothing/here")
        .with_user(r#"{"user_id":""}"#)
        .with_token(r#"{"user_id":""}"#);
    let exception = anonymous
        .run()
        .expect_err("the fallback denies an unknown route");
    assert!(exception.message.contains("Your user_id=*******"));
}

#[test]
fn direct_llm_403() {
    assert_auth_error(viewer("/v1/chat/completions", Some("POST")), "403");
    assert_auth_error(viewer("/v1/chat/completions", Some("GET")), "403");
}

#[test]
fn email_only_update() {
    viewer_body("/user/update", "POST", r#"{"user_email":"a@b.c"}"#).unwrap();
}

#[test]
fn empty_update_allowed() {
    viewer("/user/update", Some("POST")).unwrap();
}

#[test]
fn extra_update_field_403() {
    assert_auth_error(
        viewer_body(
            "/user/update",
            "POST",
            r#"{"user_email":"a@b.c","max_budget":5}"#,
        ),
        "403",
    );
    assert_auth_error(
        viewer_body("/user/update", "POST", r#"{"max_budget":5}"#),
        "403",
    );
}

#[test]
fn update_fields_checked_before_the_method() {
    // LiteLLM checks /user/update's fields in the management branch, whatever the method says
    assert_auth_error(
        viewer_body("/user/update", "GET", r#"{"max_budget":5}"#),
        "403",
    );
    // and a body on any other management route is not /user/update's to reject
    viewer_body("/team/team-1", "GET", r#"{"max_budget":5}"#).unwrap();
}

#[rstest]
#[case::user_new("/user/new")]
#[case::management_users_bulk("/management/v1/users/bulk")]
#[case::user_delete("/user/delete")]
#[case::management_users_bulk_delete("/management/v1/users/bulk_delete")]
#[case::user_bulk_update("/user/bulk_update")]
#[case::team_new("/team/new")]
#[case::management_team_members_bulk_delete("/management/v1/teams/{team_id}/members/bulk_delete")]
#[case::management_team_members_bulk_update("/management/v1/teams/{team_id}/members/bulk_update")]
#[case::team_update("/team/update")]
#[case::team_delete("/team/delete")]
#[case::team_block("/team/block")]
#[case::team_unblock("/team/unblock")]
#[case::team_permissions_update("/team/permissions_update")]
#[case::team_permissions_bulk_update("/team/permissions_bulk_update")]
#[case::model_new("/model/new")]
#[case::model_update("/model/update")]
#[case::model_delete("/model/delete")]
#[case::jwt_key_mapping_new("/jwt/key/mapping/new")]
#[case::jwt_key_mapping_update("/jwt/key/mapping/update")]
#[case::jwt_key_mapping_delete("/jwt/key/mapping/delete")]
#[case::key_generate("/key/generate")]
#[case::key_update("/key/update")]
#[case::key_delete("/key/delete")]
#[case::key_regenerate("/key/regenerate")]
#[case::key_service_account_generate("/key/service-account/generate")]
#[case::key_block("/key/block")]
#[case::key_unblock("/key/unblock")]
#[case::key_bulk_update("/key/bulk_update")]
#[case::team_key_bulk_update("/team/key/bulk_update")]
fn each_early_blocked_route(#[case] template: &str) {
    assert!(EARLY_BLOCKED_ROUTES.contains(&template));
    let management = management_routes();
    let route = template.replace("{team_id}", "team-1");
    if check_route_access(&route, management.as_slice()) {
        assert_auth_error(viewer(&route, Some("GET")), "403");
    } else {
        viewer(&route, Some("GET")).unwrap();
    }
}

#[rstest]
#[case::user_new("/user/new")]
#[case::management_users_bulk("/management/v1/users/bulk")]
#[case::user_delete("/user/delete")]
#[case::management_users_bulk_delete("/management/v1/users/bulk_delete")]
#[case::user_bulk_update("/user/bulk_update")]
#[case::team_new("/team/new")]
#[case::management_team_members_bulk_delete("/management/v1/teams/{team_id}/members/bulk_delete")]
#[case::management_team_members_bulk_update("/management/v1/teams/{team_id}/members/bulk_update")]
#[case::team_update("/team/update")]
#[case::team_delete("/team/delete")]
#[case::model_new("/model/new")]
#[case::model_update("/model/update")]
#[case::model_delete("/model/delete")]
#[case::key_generate("/key/generate")]
#[case::key_delete("/key/delete")]
#[case::key_update("/key/update")]
#[case::key_regenerate("/key/regenerate")]
#[case::key_service_account_generate("/key/service-account/generate")]
#[case::key_block("/key/block")]
#[case::key_unblock("/key/unblock")]
#[case::team_key_bulk_update("/team/key/bulk_update")]
fn each_late_blocked_route(#[case] template: &str) {
    assert!(LATE_BLOCKED_WRITE_ROUTES.contains(&template));
    let route = template.replace("{team_id}", "team-1");
    assert_auth_error(viewer(&route, Some("POST")), "403");
}

#[rstest]
#[case::regenerate("/regenerate")]
#[case::reset_spend("/reset_spend")]
fn regenerate_and_reset_suffixes(#[case] suffix: &str) {
    assert!(EARLY_BLOCKED_KEY_SUFFIXES.contains(&suffix));
    let route = format!("/key/key-1{suffix}");
    assert_auth_error(viewer(&route, Some("GET")), "403");
    assert_auth_error(viewer(&route, Some("POST")), "403");
    let late = format!("/key/key-1{LATE_BLOCKED_KEY_SUFFIX}");
    assert_auth_error(viewer(&late, Some("PATCH")), "403");
    viewer("/team/regenerate", Some("GET")).unwrap();
}

#[rstest]
#[case::get("GET")]
#[case::head("HEAD")]
#[case::options("OPTIONS")]
fn safe_get_head_options(#[case] method: &str) {
    viewer("/team/team-1", Some(method)).unwrap();
}

#[test]
fn absent_request_get() {
    viewer("/team/team-1", None).unwrap();
    viewer("/user/info", None).unwrap();
}

#[test]
fn password_change_allowed() {
    viewer("/user/password/change", Some("POST")).unwrap();
}

#[test]
fn logout_allowed() {
    viewer("/session/logout", Some("POST")).unwrap();
}

#[test]
fn legacy_read_post_allowed() {
    viewer("/spend/calculate", Some("POST")).unwrap();
    viewer("/global/spend/logs", Some("POST")).unwrap();
}

#[test]
fn unknown_post_403() {
    assert_auth_error(viewer("/nothing/here", Some("POST")), "403");
    let policy = Policy::new("/nothing/here")
        .with_method("POST")
        .with_user(VIEWER_ROW)
        .with_token(VIEWER_TOKEN);
    assert_auth_error(policy.run(), "403");
}

#[test]
fn empty_short_mask() {
    assert_eq!(mask_user_id(""), "***");
    assert_eq!(mask_user_id("a"), "***");
    assert_eq!(mask_user_id("abcd"), "***");
    assert_eq!(mask_user_id("éééé"), "***");
}

#[rstest]
#[case::five("abcde", "*****")]
#[case::six("abcdef", "******")]
#[case::seven("abcdefg", "*******")]
#[case::eight("abcdefgh", "********")]
fn five_to_eight_all_masked(#[case] user_id: &str, #[case] masked: &str) {
    assert_eq!(mask_user_id(user_id), masked);
}

#[test]
fn long_id_six_two_visible() {
    assert_eq!(mask_user_id("abcdefghi"), "abcdef*hi");
    assert_eq!(mask_user_id("alice-long-id"), "alice-*****id");
    assert_eq!(mask_user_id("ééééééééé"), "éééééé*éé");
}

#[test]
fn unknown_identity_masked() {
    let policy = Policy::new("/nothing/here")
        .with_user(r#"{"user_id":"someone-unknown"}"#)
        .with_token(r#"{"user_id":"someone-unknown"}"#);
    let exception = policy
        .run()
        .expect_err("the fallback denies an unknown route");
    assert!(exception.message.contains("Your role=unknown"));
    let masked = mask_user_id("someone-unknown");
    assert!(
        exception
            .message
            .contains(&format!("Your user_id={masked}"))
    );
}

#[rstest]
#[case::key_generate("/key/generate", true)]
#[case::team_info("/team/info", true)]
#[case::spend_logs("/global/spend/logs", true)]
#[case::chat_completions("/v1/chat/completions", false)]
fn admin_default_groups(#[case] route: &str, #[case] allowed: bool) {
    let routes = owned(&ADMIN_DEFAULT_ROUTES);
    assert_eq!(
        allowed_routes_check(UserRole::ProxyAdmin, route, &routes, None),
        allowed
    );
}

#[rstest]
#[case::chat_completions("/v1/chat/completions", true)]
#[case::key_info("/key/info", true)]
#[case::mcp_tools("/mcp/tools", true)]
#[case::messages("/v1/messages", true)]
#[case::key_generate("/key/generate", false)]
fn team_default_groups(#[case] route: &str, #[case] allowed: bool) {
    let routes = owned(&TEAM_DEFAULT_ROUTES);
    assert_eq!(
        allowed_routes_check(UserRole::Team, route, &routes, Some(routes.as_slice())),
        allowed
    );
}

#[rstest]
#[case::chat_completions("/v1/chat/completions", true)]
#[case::key_info("/key/info", true)]
#[case::mcp_tools("/mcp/tools", false)]
#[case::messages("/v1/messages", false)]
#[case::key_generate("/key/generate", false)]
fn team_none_two_group_fallback(#[case] route: &str, #[case] allowed: bool) {
    assert_eq!(
        allowed_routes_check(UserRole::Team, route, &[], None),
        allowed
    );
}

#[test]
fn empty_routes_deny() {
    let empty: Vec<String> = Vec::new();
    assert!(!allowed_routes_check(
        UserRole::ProxyAdmin,
        "/key/info",
        &empty,
        None
    ));
    assert!(!allowed_routes_check(
        UserRole::Team,
        "/key/info",
        &empty,
        Some(empty.as_slice())
    ));
}

#[rstest]
#[case::proxy_admin_viewer(UserRole::ProxyAdminViewer)]
#[case::org_admin(UserRole::OrgAdmin)]
#[case::internal_user(UserRole::InternalUser)]
#[case::internal_user_viewer(UserRole::InternalUserViewer)]
#[case::customer(UserRole::Customer)]
fn other_role_denied(#[case] role: UserRole) {
    let routes = owned(&ADMIN_DEFAULT_ROUTES);
    let team = owned(&TEAM_DEFAULT_ROUTES);
    assert!(!allowed_routes_check(
        role,
        "/key/info",
        &routes,
        Some(team.as_slice())
    ));
}

#[test]
fn route_group_template() {
    let management = owned(&["management_routes"]);
    assert!(allowed_routes_check(
        UserRole::ProxyAdmin,
        "/team/team-1",
        &management,
        None
    ));
    assert!(!allowed_routes_check(
        UserRole::ProxyAdmin,
        "/team/team-1/members",
        &management,
        None
    ));
    let self_managed = owned(&["self_managed_routes"]);
    assert!(allowed_routes_check(
        UserRole::ProxyAdmin,
        "/team/org/team-1/callback",
        &self_managed,
        None
    ));
    assert!(!allowed_routes_check(
        UserRole::ProxyAdmin,
        "/team/org/team-1/callback/extra",
        &self_managed,
        None
    ));
    // a group that expands to a single route is still matched as its member, not as the group name
    let single = owned(&["apply_guardrail_routes"]);
    assert!(allowed_routes_check(
        UserRole::ProxyAdmin,
        "/guardrails/apply_guardrail",
        &single,
        None
    ));
}

#[test]
fn route_wildcard() {
    let prefix = owned(&["/scim/v2/*"]);
    assert!(allowed_routes_check(
        UserRole::ProxyAdmin,
        "/scim/v2/Users",
        &prefix,
        None
    ));
    assert!(allowed_routes_check(
        UserRole::ProxyAdmin,
        "/scim/v2/Users/123",
        &prefix,
        None
    ));
    assert!(!allowed_routes_check(
        UserRole::ProxyAdmin,
        "/chat/completions",
        &prefix,
        None
    ));
    let everything = owned(&["*"]);
    assert!(allowed_routes_check(
        UserRole::ProxyAdmin,
        "/anything/at/all",
        &everything,
        None
    ));
}

#[test]
fn non_group_entries_are_not_templates() {
    let exact = owned(&["/internal-models"]);
    assert!(allowed_routes_check(
        UserRole::ProxyAdmin,
        "/internal-models",
        &exact,
        None
    ));
    assert!(!allowed_routes_check(
        UserRole::ProxyAdmin,
        "/internal-models/one",
        &exact,
        None
    ));
    let template = owned(&["/x/{value}"]);
    assert!(!allowed_routes_check(
        UserRole::ProxyAdmin,
        "/x/7",
        &template,
        None
    ));
    assert!(allowed_routes_check(
        UserRole::ProxyAdmin,
        "/x/{value}",
        &template,
        None
    ));
}

#[test]
fn template_text_convertor() {
    assert!(template_matches("/x/{value}", "/x/7"));
    assert!(!template_matches("/x/{value}", "/x/7/8"));
    assert!(!template_matches("/x/{value}", "/x/"));
    assert!(!template_matches("/x/{value}", "/x"));
    assert!(!template_matches("/x/{value}", "/y/7"));
    assert!(template_matches("/x/{value:str}", "/x/7"));
    assert!(!template_matches("/x/{value:str}", "/x/7/8"));
}

#[test]
fn template_path_convertor() {
    assert!(template_matches("/x/{value:path}", "/x/a/b"));
    assert!(template_matches("/x/{value:path}", "/x/"));
    assert!(template_matches("/x/{value:path}", "/x/a\n"));
    assert!(!template_matches("/x/{value:path}", "/x/a\nb"));
    assert!(!template_matches("/x/{value:path}", "/x/a\n\n"));
}

#[test]
fn template_int_and_float_convertors() {
    assert!(template_matches("/x/{value:int}", "/x/7"));
    assert!(!template_matches("/x/{value:int}", "/x/seven"));
    assert!(!template_matches("/x/{value:int}", "/x/7.5"));
    assert!(!template_matches("/x/{value:int}", "/x/"));
    assert!(template_matches("/x/{value:float}", "/x/7"));
    assert!(template_matches("/x/{value:float}", "/x/7.5"));
    assert!(!template_matches("/x/{value:float}", "/x/7."));
    assert!(!template_matches("/x/{value:float}", "/x/.5"));
    assert!(!template_matches("/x/{value:float}", "/x/seven"));
    assert!(!template_matches("/x/{value:float}", "/x/"));
    assert!(!template_matches("/x/{value:float}", "/x/7.5.6"));
}

#[test]
fn template_uuid_convertor() {
    let dashed = "/x/6f8c0e3a-2b1d-4c5e-9f00-1a2b3c4d5e6f";
    let bare = "/x/6f8c0e3a2b1d4c5e9f001a2b3c4d5e6f";
    assert!(template_matches("/x/{value:uuid}", dashed));
    assert!(template_matches("/x/{value:uuid}", bare));
    assert!(!template_matches("/x/{value:uuid}", "/x/not-a-uuid"));
    assert!(!template_matches(
        "/x/{value:uuid}",
        "/x/-6f8c0e3a-2b1d-4c5e-9f00-1a2b3c4d5e6f"
    ));
    assert!(!template_matches(
        "/x/{value:uuid}",
        "/x/6f8c0e3a2b1d4c5e9f001a2b3c4d5e6f0"
    ));
    assert!(!template_matches(
        "/x/{value:uuid}",
        "/x/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
    ));
}

#[test]
fn template_placeholders_and_literals() {
    assert!(template_matches("/x/{first}/{second}", "/x/7/8"));
    assert!(!template_matches("/x/{first}/{second}", "/x/7"));
    assert!(template_matches("/x/{a_b}/y", "/x/7/y"));
    assert!(template_matches("/x/{a1}", "/x/7"));
    assert!(template_matches("/x/{_a}", "/x/7"));
    assert!(template_matches("/x/{value}x", "/x/7x"));
    assert!(template_matches("/x/{value:path}x", "/x/a/bx"));
    assert!(!template_matches("/x/{}", "/x/7"));
    assert!(template_matches("/x/{}", "/x/{}"));
    assert!(!template_matches("/x/{:path}", "/x/7"));
    assert!(template_matches("/x/{:path}", "/x/{:path}"));
    assert!(!template_matches("/x/{1x}", "/x/7"));
    assert!(template_matches("/x/{1x}", "/x/{1x}"));
    assert!(!template_matches("/x/{value:unknown}", "/x/7"));
    assert!(template_matches("/x/{value:}", "/x/{value:}"));
    assert!(!template_matches("/x/{value:}", "/x/7"));
    assert!(template_matches("/x/{literal", "/x/{literal"));
    assert!(!template_matches("{a:int}{b:int}", "1é"));
}

#![cfg(test)]

use betterlite_config::sections::auth_user_lookup::{RbacRole, RoleBasedPermissions};
use betterlite_config::sections::jwt_settings::Settings;
use betterlite_config::sections::jwt_settings::records::ScopeMapping;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_exception::ProxyException;
use secrecy::ExposeSecret as _;

use super::admin::{AdminIdentity, check_admin_access};
use super::role::{RoleCheck, can_rbac_role_call, check_rbac_role, role_permissions};
use super::scope::check_scope_based_access;
use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::auth_user_lookup::roles::RolePermissions;
use crate::jwt_claims::TypedValue;

const CHAT: &str = "/v1/chat/completions";

fn permissions(json: &str) -> RolePermissions {
    let configured: JsonData = serde_json::from_str(json).unwrap();
    RolePermissions::from_json(&configured).unwrap()
}

fn values(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn enforced() -> Settings {
    let mut settings = Settings::default();
    settings.rbac.enforce_rbac = true;
    settings
}

fn identity() -> AdminIdentity<'static> {
    AdminIdentity {
        token: "oracle-jwt-token",
        user_id: Some("user-1"),
        user_email: Some("user@example.com"),
        org_id: Some("org-1"),
        agent_id: Some("agent-1"),
    }
}

fn wire(failure: &AuthFailure) -> ProxyException {
    as_proxy_exception(failure).exception().clone()
}

// the two model-denial call sites report the same wire fields as a route denial; the variant tells them apart
fn assert_model_denial(failure: &AuthFailure) {
    assert!(matches!(failure, AuthFailure::ModelAccessDenied { .. }));
    let exception = wire(failure);
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "403");
    assert!(exception.message.contains("not available for this API key"));
}

#[test]
fn scope_model_grant() {
    let mappings = [ScopeMapping {
        models: Some(values(&["gpt-4o"])),
        routes: None,
        scope: "chat".to_owned(),
    }];
    check_scope_based_access(Some(&mappings), &values(&["chat"]), Some("gpt-4o")).unwrap();
}

#[test]
fn scope_model_denial() {
    let mappings = [ScopeMapping {
        models: Some(values(&["gpt-4o"])),
        routes: None,
        scope: "chat".to_owned(),
    }];
    let failure =
        check_scope_based_access(Some(&mappings), &values(&["chat"]), Some("gpt-4o-mini"))
            .unwrap_err();
    assert_model_denial(&failure);
    assert!(wire(&failure).message.contains("gpt-4o-mini"));
}

#[test]
fn rbac_required() {
    let failure = check_rbac_role(
        &enforced(),
        &RolePermissions::default(),
        None,
        Some("gpt-4o"),
        CHAT,
    )
    .unwrap_err();
    assert!(matches!(
        &*failure,
        AuthFailure::Http {
            status_code: 403,
            ..
        }
    ));
    let exception = wire(&failure);
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "403");
    assert!(exception.message.contains("enforce_rbac"));
}

#[test]
fn role_route_denial() {
    let permissions = permissions(r#"[{"role":"internal_user","routes":["/user/info"]}]"#);
    let failure = check_rbac_role(
        &enforced(),
        &permissions,
        Some(RbacRole::InternalUser),
        Some("gpt-4o"),
        CHAT,
    )
    .unwrap_err();
    assert!(matches!(&*failure, AuthFailure::Http { .. }));
    let exception = wire(&failure);
    assert_eq!(exception.code, "403");
    assert!(exception.message.contains("not allowed to call route"));
}

#[test]
fn admin_route_limited() {
    let failure = check_admin_access(
        &Settings::default(),
        &values(&["litellm_proxy_admin"]),
        CHAT,
        &identity(),
        None,
    )
    .unwrap_err();
    assert!(matches!(&*failure, AuthFailure::Other { .. }));
    let exception = wire(&failure);
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "401");
    assert!(
        exception
            .message
            .starts_with("Authentication Error, Admin not allowed to access this route")
    );
}

#[test]
fn missing_model() {
    let permissions =
        permissions(r#"[{"role":"internal_user","models":["gpt-4o"],"routes":["/user/info"]}]"#);
    check_rbac_role(
        &enforced(),
        &permissions,
        Some(RbacRole::InternalUser),
        None,
        "/user/info",
    )
    .unwrap();
    can_rbac_role_call(&permissions, RbacRole::InternalUser, RoleCheck::Model(None)).unwrap();
}

#[test]
fn role_permissions_absent() {
    let permissions = RolePermissions::default();
    check_rbac_role(
        &enforced(),
        &permissions,
        Some(RbacRole::InternalUser),
        Some("any-model"),
        "/any/route",
    )
    .unwrap();
    can_rbac_role_call(&permissions, RbacRole::Team, RoleCheck::Route(CHAT)).unwrap();
}

#[test]
fn role_model_none() {
    let permissions = permissions(r#"[{"role":"internal_user","models":null}]"#);
    can_rbac_role_call(
        &permissions,
        RbacRole::InternalUser,
        RoleCheck::Model(Some("gpt-4o")),
    )
    .unwrap();
}

#[test]
fn role_model_exact() {
    let permissions = permissions(r#"[{"role":"internal_user","models":["gpt-4o"]}]"#);
    can_rbac_role_call(
        &permissions,
        RbacRole::InternalUser,
        RoleCheck::Model(Some("gpt-4o")),
    )
    .unwrap();
}

// LiteLLM checks `model is None` only, so the empty string is matched literally like any other name
#[test]
fn role_model_empty() {
    let permissions = permissions(r#"[{"role":"internal_user","models":["gpt-4o"]}]"#);
    let failure = can_rbac_role_call(
        &permissions,
        RbacRole::InternalUser,
        RoleCheck::Model(Some("")),
    )
    .unwrap_err();
    assert_model_denial(&failure);
}

#[test]
fn role_model_star_literal() {
    let permissions = permissions(r#"[{"role":"internal_user","models":["*"]}]"#);
    let failure = can_rbac_role_call(
        &permissions,
        RbacRole::InternalUser,
        RoleCheck::Model(Some("gpt-4o")),
    )
    .unwrap_err();
    assert_model_denial(&failure);
    can_rbac_role_call(
        &permissions,
        RbacRole::InternalUser,
        RoleCheck::Model(Some("*")),
    )
    .unwrap();
}

#[test]
fn role_routes_none() {
    let permissions = permissions(r#"[{"role":"team","routes":null}]"#);
    can_rbac_role_call(&permissions, RbacRole::Team, RoleCheck::Route(CHAT)).unwrap();
}

#[test]
fn role_route_group() {
    let permissions = permissions(r#"[{"role":"team","routes":["openai_routes"]}]"#);
    can_rbac_role_call(&permissions, RbacRole::Team, RoleCheck::Route(CHAT)).unwrap();
    let failure = can_rbac_role_call(&permissions, RbacRole::Team, RoleCheck::Route("/user/info"))
        .unwrap_err();
    assert!(matches!(&*failure, AuthFailure::Http { .. }));
}

#[test]
fn scope_empty_mapping() {
    check_scope_based_access(Some(&[]), &values(&["chat"]), Some("gpt-4o")).unwrap();
    check_scope_based_access(None, &values(&["chat"]), Some("gpt-4o")).unwrap();
}

#[test]
fn scope_missing_model() {
    let mappings = [ScopeMapping {
        models: Some(values(&["gpt-4o"])),
        routes: None,
        scope: "chat".to_owned(),
    }];
    check_scope_based_access(Some(&mappings), &values(&["chat"]), None).unwrap();
    check_scope_based_access(Some(&mappings), &values(&["chat"]), Some("")).unwrap();
}

#[test]
fn scope_union() {
    let mappings = [
        ScopeMapping {
            models: Some(values(&["gpt-4o"])),
            routes: None,
            scope: "chat".to_owned(),
        },
        ScopeMapping {
            models: Some(values(&["claude-3"])),
            routes: None,
            scope: "chat".to_owned(),
        },
    ];
    check_scope_based_access(Some(&mappings), &values(&["chat"]), Some("claude-3")).unwrap();
    check_scope_based_access(Some(&mappings), &values(&["chat"]), Some("gpt-4o")).unwrap();
}

#[test]
fn scope_unmatched() {
    let mappings = [ScopeMapping {
        models: Some(values(&["gpt-4o"])),
        routes: None,
        scope: "responses".to_owned(),
    }];
    let failure =
        check_scope_based_access(Some(&mappings), &values(&["chat"]), Some("gpt-4o")).unwrap_err();
    assert_model_denial(&failure);
}

#[test]
fn scope_star_literal() {
    let mappings = [ScopeMapping {
        models: Some(values(&["*"])),
        routes: None,
        scope: "chat".to_owned(),
    }];
    let failure =
        check_scope_based_access(Some(&mappings), &values(&["chat"]), Some("gpt-4o")).unwrap_err();
    assert_model_denial(&failure);
    check_scope_based_access(Some(&mappings), &values(&["chat"]), Some("*")).unwrap();
}

#[test]
fn role_model_denial() {
    let permissions = permissions(r#"[{"role":"internal_user","models":["gpt-4o"]}]"#);
    let failure = can_rbac_role_call(
        &permissions,
        RbacRole::InternalUser,
        RoleCheck::Model(Some("gpt-4o-mini")),
    )
    .unwrap_err();
    assert_model_denial(&failure);
}

// general_settings.role_permissions reaches the checks through the same conversion the server uses
#[test]
fn configured_role_permissions() {
    let entries: Vec<RoleBasedPermissions> = serde_json::from_str(
        r#"[{"role":"internal_user","models":["gpt-4o"],"routes":["/user/info"]}]"#,
    )
    .unwrap();
    let permissions = role_permissions(Some(&entries)).unwrap();
    check_rbac_role(
        &enforced(),
        &permissions,
        Some(RbacRole::InternalUser),
        Some("gpt-4o"),
        "/user/info",
    )
    .unwrap();
    let failure = check_rbac_role(
        &enforced(),
        &permissions,
        Some(RbacRole::InternalUser),
        Some("gpt-4o-mini"),
        "/user/info",
    )
    .unwrap_err();
    assert_model_denial(&failure);
}

#[test]
fn rbac_disabled() {
    let permissions =
        permissions(r#"[{"role":"internal_user","models":["gpt-4o"],"routes":["/user/info"]}]"#);
    check_rbac_role(
        &Settings::default(),
        &permissions,
        None,
        Some("gpt-4o-mini"),
        CHAT,
    )
    .unwrap();
}

#[test]
fn model_before_route() {
    let permissions =
        permissions(r#"[{"role":"internal_user","models":["gpt-4o"],"routes":["/user/info"]}]"#);
    let failure = check_rbac_role(
        &enforced(),
        &permissions,
        Some(RbacRole::InternalUser),
        Some("gpt-4o-mini"),
        CHAT,
    )
    .unwrap_err();
    assert_model_denial(&failure);
}

#[test]
fn admin_not_scoped() {
    let outcome = check_admin_access(
        &Settings::default(),
        &values(&["chat"]),
        CHAT,
        &identity(),
        None,
    )
    .unwrap();
    assert!(outcome.is_none());
}

// is_admin is literal membership: the scope among others still counts, and a token with no scopes never does
#[test]
fn admin_scope_membership() {
    let mixed = check_admin_access(
        &Settings::default(),
        &values(&["chat", "litellm_proxy_admin"]),
        "/user/info",
        &identity(),
        None,
    )
    .unwrap();
    assert!(mixed.is_some());
    let unscoped =
        check_admin_access(&Settings::default(), &[], "/user/info", &identity(), None).unwrap();
    assert!(unscoped.is_none());
}

#[test]
fn admin_result() {
    let claims: TypedValue = serde_json::from_str(r#"{"sub": "user-1", "custom": true}"#).unwrap();
    let result = check_admin_access(
        &Settings::default(),
        &values(&["litellm_proxy_admin"]),
        "/user/info",
        &identity(),
        Some(&claims),
    )
    .unwrap()
    .unwrap();
    assert!(result.is_proxy_admin);
    assert_eq!(result.token.expose_secret(), "oracle-jwt-token");
    assert_eq!(result.user_id.as_deref(), Some("user-1"));
    assert_eq!(result.user_email.as_deref(), Some("user@example.com"));
    assert_eq!(result.org_id.as_deref(), Some("org-1"));
    assert_eq!(result.agent_id.as_deref(), Some("agent-1"));
    assert!(result.team_id.is_none());
    assert!(result.end_user_id.is_none());
    assert_eq!(result.jwt_claims, claims);
    let without_claims = check_admin_access(
        &Settings::default(),
        &values(&["litellm_proxy_admin"]),
        "/user/info",
        &identity(),
        None,
    )
    .unwrap()
    .unwrap();
    let empty: TypedValue = serde_json::from_str("{}").unwrap();
    assert_eq!(without_claims.jwt_claims, empty);
}

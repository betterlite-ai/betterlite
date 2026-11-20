use std::collections::BTreeMap;

use betterlite_config::sections::auth_user_lookup::RbacRole;
use betterlite_db::budget_permission_rows::JsonData;

use crate::auth_user_lookup::roles::RolePermissions;

fn strings(values: &[&str]) -> JsonData {
    JsonData::Array(
        values
            .iter()
            .map(|value| JsonData::String((*value).to_owned()))
            .collect(),
    )
}

fn role_entry(role: &str, models: Option<&[&str]>, routes: Option<&[&str]>) -> JsonData {
    let mut fields = BTreeMap::new();
    fields.insert("role".to_owned(), JsonData::String(role.to_owned()));
    if let Some(model_list) = models {
        fields.insert("models".to_owned(), strings(model_list));
    }
    if let Some(route_list) = routes {
        fields.insert("routes".to_owned(), strings(route_list));
    }
    JsonData::Object(fields)
}

fn owned(values: Option<&[String]>) -> Option<Vec<String>> {
    values.map(ToOwned::to_owned)
}

fn parse_role(role: &str) -> Result<RbacRole, serde_json::Error> {
    serde_json::from_str(&format!("\"{role}\""))
}

fn models(permissions: &RolePermissions, role: RbacRole) -> Option<&[String]> {
    let entry = permissions.selected(role)?;
    entry.models.as_deref()
}

fn routes(permissions: &RolePermissions, role: RbacRole) -> Option<&[String]> {
    let entry = permissions.selected(role)?;
    entry.routes.as_deref()
}

#[test]
fn role_defaults_none() {
    let empty = RolePermissions::default();
    assert_eq!(models(&empty, RbacRole::ProxyAdmin), None);
    assert_eq!(routes(&empty, RbacRole::Team), None);
    let null = RolePermissions::from_json(&JsonData::Null).unwrap();
    assert_eq!(models(&null, RbacRole::InternalUser), None);
    assert_eq!(routes(&null, RbacRole::ProxyAdmin), None);
    assert_eq!(models(&empty, RbacRole::Team), None);
}

#[test]
fn three_role_values_only() {
    assert_eq!(parse_role("proxy_admin").unwrap(), RbacRole::ProxyAdmin);
    assert_eq!(parse_role("team").unwrap(), RbacRole::Team);
    assert_eq!(parse_role("internal_user").unwrap(), RbacRole::InternalUser);
    parse_role("admin").unwrap_err();
    parse_role("Proxy_Admin").unwrap_err();
    parse_role("").unwrap_err();
}

#[test]
fn first_match_including_none_or_empty() {
    let configured = JsonData::Array(vec![
        role_entry("proxy_admin", Some(&["gpt-4"]), None),
        role_entry("proxy_admin", Some(&["other"]), Some(&["/user/new"])),
        role_entry("team", None, Some(&[])),
        role_entry("team", Some(&["later"]), Some(&["/later"])),
    ]);
    let roles = RolePermissions::from_json(&configured).unwrap();
    assert_eq!(
        owned(models(&roles, RbacRole::ProxyAdmin)),
        Some(vec!["gpt-4".to_owned()])
    );
    assert_eq!(owned(routes(&roles, RbacRole::ProxyAdmin)), None);
    assert_eq!(owned(models(&roles, RbacRole::Team)), None);
    assert_eq!(owned(routes(&roles, RbacRole::Team)), Some(Vec::new()));
}

#[test]
fn whole_list_validated_before_match() {
    let configured = JsonData::Array(vec![
        role_entry("proxy_admin", Some(&["gpt-4"]), None),
        role_entry("admin", None, None),
    ]);
    RolePermissions::from_json(&configured).unwrap_err();
    let missing_role = JsonData::Array(vec![JsonData::Object(BTreeMap::from([(
        "models".to_owned(),
        strings(&["gpt-4"]),
    )]))]);
    RolePermissions::from_json(&missing_role).unwrap_err();
}

#[test]
fn role_extras_rejected() {
    let JsonData::Object(mut entry) = role_entry("proxy_admin", None, None) else {
        panic!("a role entry is a mapping");
    };
    entry.insert("extra".to_owned(), JsonData::Bool(true));
    let configured = JsonData::Array(vec![JsonData::Object(entry)]);
    RolePermissions::from_json(&configured).unwrap_err();
}

#[test]
fn role_no_match() {
    let configured = JsonData::Array(vec![role_entry(
        "team",
        Some(&["gpt-4"]),
        Some(&["/team/info"]),
    )]);
    let roles = RolePermissions::from_json(&configured).unwrap();
    assert_eq!(owned(models(&roles, RbacRole::ProxyAdmin)), None);
    assert_eq!(owned(routes(&roles, RbacRole::InternalUser)), None);
}

#[test]
fn permission_order_duplicates_and_empty_strings() {
    let configured = JsonData::Array(vec![role_entry(
        "internal_user",
        Some(&["b", "a", "b", ""]),
        Some(&["/b", "/b", ""]),
    )]);
    let roles = RolePermissions::from_json(&configured).unwrap();
    assert_eq!(
        owned(models(&roles, RbacRole::InternalUser)),
        Some(vec![
            "b".to_owned(),
            "a".to_owned(),
            "b".to_owned(),
            String::new(),
        ])
    );
    assert_eq!(
        owned(routes(&roles, RbacRole::InternalUser)),
        Some(vec!["/b".to_owned(), "/b".to_owned(), String::new()])
    );
}

#[test]
fn models_routes_select_independently() {
    let configured = JsonData::Array(vec![
        role_entry("proxy_admin", Some(&["gpt-4"]), None),
        role_entry("team", None, Some(&["/team/info"])),
        role_entry("proxy_admin", Some(&["later"]), Some(&["/user/new"])),
    ]);
    let roles = RolePermissions::from_json(&configured).unwrap();
    assert_eq!(
        owned(models(&roles, RbacRole::ProxyAdmin)),
        Some(vec!["gpt-4".to_owned()])
    );
    assert_eq!(owned(routes(&roles, RbacRole::ProxyAdmin)), None);
    assert_eq!(owned(models(&roles, RbacRole::Team)), None);
    assert_eq!(
        owned(routes(&roles, RbacRole::Team)),
        Some(vec!["/team/info".to_owned()])
    );
    assert_eq!(owned(models(&roles, RbacRole::InternalUser)), None);
    assert_eq!(owned(routes(&roles, RbacRole::InternalUser)), None);
}

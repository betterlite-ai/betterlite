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
    if let Some(models) = models {
        fields.insert("models".to_owned(), strings(models));
    }
    if let Some(routes) = routes {
        fields.insert("routes".to_owned(), strings(routes));
    }
    JsonData::Object(fields)
}

fn owned(values: Option<&[String]>) -> Option<Vec<String>> {
    values.map(ToOwned::to_owned)
}

fn parse_role(role: &str) -> Result<RbacRole, serde_json::Error> {
    serde_json::from_str(&format!("\"{role}\""))
}

#[test]
fn role_defaults_none() {
    let empty = RolePermissions::default();
    assert_eq!(empty.models(RbacRole::ProxyAdmin), None);
    assert_eq!(empty.routes(RbacRole::Team), None);
    let null = RolePermissions::from_json(&JsonData::Null).unwrap();
    assert_eq!(null.models(RbacRole::InternalUser), None);
    assert_eq!(null.routes(RbacRole::ProxyAdmin), None);
    let unconfigured = RolePermissions::configured(None);
    assert_eq!(unconfigured.models(RbacRole::Team), None);
}

#[test]
fn three_role_values_only() {
    assert_eq!(parse_role("proxy_admin").unwrap(), RbacRole::ProxyAdmin);
    assert_eq!(parse_role("team").unwrap(), RbacRole::Team);
    assert_eq!(parse_role("internal_user").unwrap(), RbacRole::InternalUser);
    assert!(matches!(parse_role("admin"), Err(_)));
    assert!(matches!(parse_role("Proxy_Admin"), Err(_)));
    assert!(matches!(parse_role(""), Err(_)));
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
        owned(roles.models(RbacRole::ProxyAdmin)),
        Some(vec!["gpt-4".to_owned()])
    );
    assert_eq!(owned(roles.routes(RbacRole::ProxyAdmin)), None);
    assert_eq!(owned(roles.models(RbacRole::Team)), None);
    assert_eq!(owned(roles.routes(RbacRole::Team)), Some(Vec::new()));
}

#[test]
fn whole_list_validated_before_match() {
    let configured = JsonData::Array(vec![
        role_entry("proxy_admin", Some(&["gpt-4"]), None),
        role_entry("admin", None, None),
    ]);
    assert!(matches!(RolePermissions::from_json(&configured), Err(_)));
    let missing_role = JsonData::Array(vec![JsonData::Object(BTreeMap::from([(
        "models".to_owned(),
        strings(&["gpt-4"]),
    )]))]);
    assert!(matches!(RolePermissions::from_json(&missing_role), Err(_)));
}

#[test]
fn role_extras_rejected() {
    let JsonData::Object(mut entry) = role_entry("proxy_admin", None, None) else {
        panic!("a role entry is a mapping");
    };
    entry.insert("extra".to_owned(), JsonData::Bool(true));
    let configured = JsonData::Array(vec![JsonData::Object(entry)]);
    assert!(matches!(RolePermissions::from_json(&configured), Err(_)));
}

#[test]
fn role_no_match() {
    let configured = JsonData::Array(vec![role_entry(
        "team",
        Some(&["gpt-4"]),
        Some(&["/team/info"]),
    )]);
    let roles = RolePermissions::from_json(&configured).unwrap();
    assert_eq!(owned(roles.models(RbacRole::ProxyAdmin)), None);
    assert_eq!(owned(roles.routes(RbacRole::InternalUser)), None);
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
        owned(roles.models(RbacRole::InternalUser)),
        Some(vec![
            "b".to_owned(),
            "a".to_owned(),
            "b".to_owned(),
            String::new(),
        ])
    );
    assert_eq!(
        owned(roles.routes(RbacRole::InternalUser)),
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
        owned(roles.models(RbacRole::ProxyAdmin)),
        Some(vec!["gpt-4".to_owned()])
    );
    assert_eq!(owned(roles.routes(RbacRole::ProxyAdmin)), None);
    assert_eq!(owned(roles.models(RbacRole::Team)), None);
    assert_eq!(
        owned(roles.routes(RbacRole::Team)),
        Some(vec!["/team/info".to_owned()])
    );
    assert_eq!(owned(roles.models(RbacRole::InternalUser)), None);
    assert_eq!(owned(roles.routes(RbacRole::InternalUser)), None);
}

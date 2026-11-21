#![cfg(test)]

mod loader;
mod nested;
mod order;

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::Settings;
use super::construct::{Context, RawJwtAuth, construct};
use super::error::JwtSettingsError;
use super::groups::DEFAULT_JWKS_STALE_TTL;
use super::records::UnregisteredClientBehavior;
use super::validator::UnregisteredLoader;

fn parsed(yaml: &str) -> RawJwtAuth {
    serde_saphyr::from_str(yaml).unwrap()
}

fn build(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, JwtSettingsError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let context = Context {
        loader: &UnregisteredLoader,
        config_file_path: None,
        lookup: &lookup,
    };
    construct(parsed(yaml), &context)
}

fn dump(settings: &Settings) -> BTreeMap<String, JsonData> {
    serde_json::from_str(&serde_json::to_string(settings).unwrap()).unwrap()
}

fn field(yaml: &str, environment: &[(&str, &str)], name: &str) -> JsonData {
    dump(&build(yaml, environment).unwrap())
        .remove(name)
        .unwrap()
}

fn rejection(yaml: &str, environment: &[(&str, &str)]) -> String {
    build(yaml, environment).unwrap_err().to_string()
}

fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

fn number(value: f64) -> JsonData {
    JsonData::Number(serde_json::Number::from_f64(value).unwrap())
}

fn texts(values: &[&str]) -> JsonData {
    JsonData::Array(values.iter().map(|value| text(value)).collect())
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

#[test]
fn absent_settings_take_every_default() {
    let built = build("{}", &[]).unwrap();
    assert_eq!(built, Settings::default());
    let defaults = dump(&built);
    assert_eq!(defaults.len(), 43);
    assert_eq!(defaults["admin_jwt_scope"], text("litellm_proxy_admin"));
    assert_eq!(
        defaults["admin_allowed_routes"],
        texts(&[
            "management_routes",
            "spend_tracking_routes",
            "global_spend_tracking_routes",
            "info_routes"
        ])
    );
    assert_eq!(
        defaults["team_allowed_routes"],
        texts(&[
            "openai_routes",
            "info_routes",
            "mcp_routes",
            "/v1/messages",
            "/v1/messages/count_tokens"
        ])
    );
    assert_eq!(defaults["public_allowed_routes"], texts(&["public_routes"]));
    assert_eq!(defaults["public_key_ttl"], number(600.0));
    assert_eq!(defaults["public_key_stale_ttl"], number(3600.0));
    assert_eq!(DEFAULT_JWKS_STALE_TTL.to_bits(), 3600.0_f64.to_bits());
    assert_eq!(defaults["oidc_userinfo_cache_ttl"], number(300.0));
    assert_eq!(defaults["virtual_key_mapping_cache_ttl"], number(300.0));
    assert_eq!(
        defaults["unregistered_jwt_client_behavior"],
        text("fallback_team_mapping")
    );
    assert_eq!(defaults["custom_validate"], JsonData::Null);
    assert_eq!(defaults["issuers"], JsonData::Null);
}

#[rstest]
#[case::field_team_id_jwt_field("team_id_jwt_field")]
#[case::field_team_ids_jwt_field("team_ids_jwt_field")]
#[case::field_team_id_default("team_id_default")]
#[case::field_team_alias_jwt_field("team_alias_jwt_field")]
#[case::field_org_id_jwt_field("org_id_jwt_field")]
#[case::field_org_alias_jwt_field("org_alias_jwt_field")]
#[case::field_user_id_jwt_field("user_id_jwt_field")]
#[case::field_user_email_jwt_field("user_email_jwt_field")]
#[case::field_user_allowed_email_domain("user_allowed_email_domain")]
#[case::field_end_user_id_jwt_field("end_user_id_jwt_field")]
#[case::field_agent_id_jwt_field("agent_id_jwt_field")]
#[case::field_mcp_client_id_jwt_field("mcp_client_id_jwt_field")]
#[case::field_roles_jwt_field("roles_jwt_field")]
#[case::field_oidc_userinfo_endpoint("oidc_userinfo_endpoint")]
#[case::field_virtual_key_claim_field("virtual_key_claim_field")]
fn optional_text_rows(#[case] name: &str) {
    let reference = format!("{{{name}: os.environ/CLAIM}}");
    let accepted = [
        field("{}", &[], name),
        field(&format!("{{{name}: null}}"), &[], name),
        field(&format!("{{{name}: ''}}"), &[], name),
        field(&format!("{{{name}: realm.claim}}"), &[], name),
        field(&reference, &[("CLAIM", "sub")], name),
        field(&reference, &[("CLAIM", "")], name),
    ];
    let expected = [
        JsonData::Null,
        JsonData::Null,
        text(""),
        text("realm.claim"),
        text("sub"),
        JsonData::Null,
    ];
    assert_eq!(accepted, expected);
    let rejected = [
        rejection(&reference, &[]),
        rejection(&format!("{{{name}: 5}}"), &[]),
        rejection(&format!("{{{name}: [sub]}}"), &[]),
        rejection(&format!("{{{name}: true}}"), &[]),
    ];
    let invalid = format!("invalid value at {name}");
    let unset = format!("environment variable CLAIM for {name} is not set");
    assert_eq!(rejected, [unset, invalid.clone(), invalid.clone(), invalid]);
}

#[rstest]
#[case::field_team_id_upsert("team_id_upsert")]
#[case::field_upsert_sso_user_to_team("upsert_sso_user_to_team")]
#[case::field_user_id_upsert("user_id_upsert")]
#[case::field_enforce_rbac("enforce_rbac")]
#[case::field_enforce_team_based_model_access("enforce_team_based_model_access")]
#[case::field_sync_user_role_and_teams("sync_user_role_and_teams")]
#[case::field_oidc_userinfo_enabled("oidc_userinfo_enabled")]
#[case::field_team_claim_fallback("team_claim_fallback")]
#[case::field_fallback_to_db_teams("fallback_to_db_teams")]
fn flag_rows(#[case] name: &str) {
    assert_eq!(field("{}", &[], name), JsonData::Bool(false));
    assert_eq!(
        field(&format!("{{{name}: null}}"), &[], name),
        JsonData::Bool(false)
    );
    assert_eq!(
        field(&format!("{{{name}: true}}"), &[], name),
        JsonData::Bool(true)
    );
    assert_eq!(
        field(&format!("{{{name}: false}}"), &[], name),
        JsonData::Bool(false)
    );
    let reference = format!("{{{name}: os.environ/FLAG}}");
    assert_eq!(
        field(&reference, &[("FLAG", "on")], name),
        JsonData::Bool(true)
    );
    assert_eq!(
        field(&reference, &[("FLAG", "false")], name),
        JsonData::Bool(false)
    );
    assert_eq!(
        field(&reference, &[("FLAG", "")], name),
        JsonData::Bool(false)
    );
    assert_eq!(
        rejection(&reference, &[("FLAG", "maybe")]),
        "environment variable FLAG must be a bool"
    );
    let invalid = format!("invalid value at {name}");
    assert_eq!(rejection(&format!("{{{name}: 'yes'}}"), &[]), invalid);
    assert_eq!(rejection(&format!("{{{name}: 1}}"), &[]), invalid);
    assert_eq!(rejection(&format!("{{{name}: [true]}}"), &[]), invalid);
}

#[rstest]
#[case::field_public_key_ttl("public_key_ttl", 600.0_f64)]
#[case::field_oidc_userinfo_cache_ttl("oidc_userinfo_cache_ttl", 300.0_f64)]
#[case::field_virtual_key_mapping_cache_ttl("virtual_key_mapping_cache_ttl", 300.0_f64)]
fn unbounded_ttl_rows(#[case] name: &str, #[case] default: f64) {
    assert_eq!(field("{}", &[], name), number(default));
    assert_eq!(
        field(&format!("{{{name}: null}}"), &[], name),
        number(default)
    );
    assert_eq!(field(&format!("{{{name}: 0}}"), &[], name), number(0.0));
    assert_eq!(field(&format!("{{{name}: -5}}"), &[], name), number(-5.0));
    assert_eq!(field(&format!("{{{name}: 1.5}}"), &[], name), number(1.5));
    let reference = format!("{{{name}: os.environ/TTL}}");
    assert_eq!(field(&reference, &[("TTL", "42")], name), number(42.0));
    assert_eq!(field(&reference, &[("TTL", "")], name), number(default));
    let invalid = format!("invalid value at {name}");
    assert_eq!(rejection(&format!("{{{name}: 'ten'}}"), &[]), invalid);
    assert_eq!(rejection(&format!("{{{name}: true}}"), &[]), invalid);
}

#[rstest]
#[case::field_public_key_stale_ttl_default("{}", Some(3_600.0_f64))]
#[case::field_public_key_stale_ttl_null("{public_key_stale_ttl: null}", Some(3_600.0_f64))]
#[case::field_public_key_stale_ttl_zero("{public_key_stale_ttl: 0}", Some(0.0_f64))]
#[case::field_public_key_stale_ttl_positive("{public_key_stale_ttl: 0.5}", Some(0.5_f64))]
#[case::field_public_key_stale_ttl_negative("{public_key_stale_ttl: -1}", None)]
#[case::field_public_key_stale_ttl_small_negative("{public_key_stale_ttl: -0.001}", None)]
#[case::field_public_key_stale_ttl_text("{public_key_stale_ttl: 'hour'}", None)]
fn stale_ttl_rows(#[case] yaml: &str, #[case] expected: Option<f64>) {
    match expected {
        Some(ttl) => assert_eq!(
            build(yaml, &[])
                .unwrap()
                .keys
                .public_key_stale_ttl
                .to_bits(),
            ttl.to_bits()
        ),
        None => assert_eq!(
            rejection(yaml, &[]),
            "invalid value at public_key_stale_ttl"
        ),
    }
}

#[rstest]
#[case::field_admin_allowed_routes(
    "admin_allowed_routes",
    &["management_routes", "spend_tracking_routes", "global_spend_tracking_routes", "info_routes"]
)]
#[case::field_team_allowed_routes(
    "team_allowed_routes",
    &["openai_routes", "info_routes", "mcp_routes", "/v1/messages", "/v1/messages/count_tokens"]
)]
#[case::field_public_allowed_routes("public_allowed_routes", &["public_routes"])]
fn route_list_rows(#[case] name: &str, #[case] default: &[&str]) {
    assert_eq!(field("{}", &[], name), texts(default));
    assert_eq!(
        field(&format!("{{{name}: null}}"), &[], name),
        texts(default)
    );
    assert_eq!(field(&format!("{{{name}: []}}"), &[], name), texts(&[]));
    assert_eq!(
        field(&format!("{{{name}: [info_routes, /v1/models]}}"), &[], name),
        texts(&["info_routes", "/v1/models"])
    );
    let reference = format!("{{{name}: [os.environ/ROUTE]}}");
    assert_eq!(
        field(&reference, &[("ROUTE", "x")], name),
        texts(&["os.environ/ROUTE"])
    );
    assert_eq!(
        rejection(&format!("{{{name}: info_routes}}"), &[]),
        format!("invalid value at {name}")
    );
    assert_eq!(
        rejection(&format!("{{{name}: [info_routes, 5]}}"), &[]),
        format!("invalid value at {name}.1")
    );
    assert_eq!(
        rejection(&format!("{{{name}: [null]}}"), &[]),
        format!("invalid value at {name}.0")
    );
}

#[rstest]
#[case::field_admin_jwt_scope_default("{}", "litellm_proxy_admin")]
#[case::field_admin_jwt_scope_null("{admin_jwt_scope: null}", "litellm_proxy_admin")]
#[case::field_admin_jwt_scope_empty("{admin_jwt_scope: ''}", "")]
#[case::field_admin_jwt_scope_explicit("{admin_jwt_scope: admin.scope}", "admin.scope")]
#[case::field_admin_jwt_scope_reference("{admin_jwt_scope: os.environ/SCOPE}", "env.scope")]
fn admin_scope_rows(#[case] yaml: &str, #[case] expected: &str) {
    assert_eq!(
        field(yaml, &[("SCOPE", "env.scope")], "admin_jwt_scope"),
        text(expected)
    );
    assert_eq!(
        rejection("{admin_jwt_scope: 5}", &[]),
        "invalid value at admin_jwt_scope"
    );
}

#[rstest]
#[case::field_unregistered_jwt_client_behavior_default(
    "{}",
    Some(UnregisteredClientBehavior::FallbackTeamMapping)
)]
#[case::field_unregistered_jwt_client_behavior_null(
    "{unregistered_jwt_client_behavior: null}",
    Some(UnregisteredClientBehavior::FallbackTeamMapping)
)]
#[case::field_unregistered_jwt_client_behavior_fallback(
    "{unregistered_jwt_client_behavior: fallback_team_mapping}",
    Some(UnregisteredClientBehavior::FallbackTeamMapping)
)]
#[case::field_unregistered_jwt_client_behavior_reject(
    "{unregistered_jwt_client_behavior: reject}",
    Some(UnregisteredClientBehavior::Reject)
)]
#[case::field_unregistered_jwt_client_behavior_auto_register(
    "{unregistered_jwt_client_behavior: auto_register}",
    Some(UnregisteredClientBehavior::AutoRegister)
)]
#[case::field_unregistered_jwt_client_behavior_reference(
    "{unregistered_jwt_client_behavior: os.environ/BEHAVIOR}",
    Some(UnregisteredClientBehavior::Reject)
)]
#[case::field_unregistered_jwt_client_behavior_unknown(
    "{unregistered_jwt_client_behavior: deny}",
    None
)]
#[case::field_unregistered_jwt_client_behavior_upper(
    "{unregistered_jwt_client_behavior: REJECT}",
    None
)]
#[case::field_unregistered_jwt_client_behavior_empty(
    "{unregistered_jwt_client_behavior: ''}",
    None
)]
#[case::field_unregistered_jwt_client_behavior_number(
    "{unregistered_jwt_client_behavior: 1}",
    None
)]
fn unregistered_behavior_rows(
    #[case] yaml: &str,
    #[case] expected: Option<UnregisteredClientBehavior>,
) {
    let outcome = build(yaml, &[("BEHAVIOR", "reject")])
        .map(|built| built.virtual_keys.unregistered_client)
        .map_err(|error| error.to_string());
    let wanted =
        expected.ok_or_else(|| "invalid value at unregistered_jwt_client_behavior".to_owned());
    assert_eq!(outcome, wanted);
}

#[rstest]
#[case::field_custom_validate_default("{}")]
#[case::field_custom_validate_null("{custom_validate: null}")]
#[case::field_custom_validate_empty_reference("{custom_validate: os.environ/VALIDATOR}")]
fn custom_validate_rows(#[case] yaml: &str) {
    let built = build(yaml, &[("VALIDATOR", "")]).unwrap();
    assert_eq!(built.custom_validate, None);
    assert_eq!(
        rejection("{custom_validate: 5}", &[]),
        "invalid value at custom_validate"
    );
}

#[test]
fn ignored_record_keys_are_logged() {
    let yaml = "{role_mappings: [{role: admin, internal_role: proxy_admin, note: x}], jwt_litellm_role_map: [{jwt_role: a, litellm_role: team, weight: 1}]}";
    let (built, log) = logged(|| build(yaml, &[]));
    assert_eq!(built.unwrap().rbac.role_mappings.unwrap().len(), 1);
    assert!(log.contains("role_mappings.0.note"), "{log}");
    assert!(log.contains("jwt_litellm_role_map.0.weight"), "{log}");
}

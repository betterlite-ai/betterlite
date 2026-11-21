use rstest::rstest;

use super::super::error::ALLOWED_KEYS;
use super::{build, rejection, text};
use betterlite_db::budget_permission_rows::JsonData;

const ROLES_PAIR: &str = "user_allowed_roles must be provided if user_roles_jwt_field is set.";
const OBJECT_ID: &str = "if object_id_jwt_field is set, role_mappings must also be set. Needed to infer if the caller is a user or team.";
const SCOPES: &str = "scope_mappings must be set if enforce_scope_based_access is true.";

#[rstest]
#[case::field_user_roles_jwt_field_alone("{user_roles_jwt_field: roles}", false)]
#[case::field_user_allowed_roles_alone("{user_allowed_roles: [admin]}", false)]
#[case::field_user_roles_jwt_field_with_null_pair(
    "{user_roles_jwt_field: roles, user_allowed_roles: null}",
    false
)]
#[case::field_user_allowed_roles_with_null_pair(
    "{user_roles_jwt_field: null, user_allowed_roles: [admin]}",
    false
)]
#[case::field_user_roles_jwt_field_invalid_alone("{user_roles_jwt_field: 5}", false)]
#[case::field_user_allowed_roles_invalid_alone("{user_allowed_roles: admin}", false)]
#[case::field_user_roles_jwt_field_both(
    "{user_roles_jwt_field: roles, user_allowed_roles: [admin]}",
    true
)]
#[case::field_user_allowed_roles_empty_pair(
    "{user_roles_jwt_field: '', user_allowed_roles: []}",
    true
)]
#[case::field_user_roles_jwt_field_neither("{}", true)]
#[case::field_user_roles_jwt_field_empty_reference(
    "{user_roles_jwt_field: os.environ/EMPTY}",
    true
)]
#[case::field_user_roles_jwt_field_unset_reference(
    "{user_roles_jwt_field: os.environ/UNSET}",
    false
)]
#[case::field_user_roles_jwt_field_set_reference("{user_roles_jwt_field: os.environ/ROLES}", false)]
fn paired_role_rows(#[case] yaml: &str, #[case] accepted: bool) {
    let built = build(yaml, &[("EMPTY", ""), ("ROLES", "roles")]);
    if accepted {
        built.unwrap();
    } else {
        let message = built.unwrap_err().to_string();
        assert!(
            message == ROLES_PAIR || message.contains("UNSET"),
            "{message}"
        );
    }
}

#[test]
fn paired_role_values_load() {
    let built = build(
        "{user_roles_jwt_field: realm.roles, user_allowed_roles: [admin, ops]}",
        &[],
    )
    .unwrap();
    assert_eq!(
        built.claims.user_roles_jwt_field.as_deref(),
        Some("realm.roles")
    );
    assert_eq!(
        built.claims.user_allowed_roles,
        Some(vec!["admin".to_owned(), "ops".to_owned()])
    );
    assert_eq!(
        build("{user_roles_jwt_field: os.environ/UNSET}", &[])
            .unwrap_err()
            .to_string(),
        "environment variable UNSET for user_roles_jwt_field is not set"
    );
}

#[rstest]
#[case::field_object_id_jwt_field_without_mappings("{object_id_jwt_field: oid}", false)]
#[case::field_object_id_jwt_field_null_mappings(
    "{object_id_jwt_field: oid, role_mappings: null}",
    false
)]
#[case::field_object_id_jwt_field_empty("{object_id_jwt_field: '', role_mappings: null}", false)]
#[case::field_object_id_jwt_field_invalid("{object_id_jwt_field: 5}", false)]
#[case::field_object_id_jwt_field_empty_mappings(
    "{object_id_jwt_field: oid, role_mappings: []}",
    true
)]
#[case::field_object_id_jwt_field_null("{object_id_jwt_field: null}", true)]
#[case::field_object_id_jwt_field_empty_reference("{object_id_jwt_field: os.environ/EMPTY}", true)]
#[case::field_role_mappings_without_object_id(
    "{role_mappings: [{role: admin, internal_role: proxy_admin}]}",
    true
)]
fn object_id_rows(#[case] yaml: &str, #[case] accepted: bool) {
    let built = build(yaml, &[("EMPTY", "")]);
    if accepted {
        built.unwrap();
    } else {
        assert_eq!(built.unwrap_err().to_string(), OBJECT_ID);
    }
}

#[rstest]
#[case::field_scope_mappings_enforced(
    "{scope_mappings: [{scope: read}], enforce_scope_based_access: true}",
    Some(true)
)]
#[case::field_scope_mappings_empty_enforced(
    "{scope_mappings: [], enforce_scope_based_access: true}",
    Some(true)
)]
#[case::field_enforce_scope_based_access_without_mappings(
    "{enforce_scope_based_access: true}",
    Some(true)
)]
#[case::field_scope_mappings_not_enforced(
    "{scope_mappings: [{scope: read}], enforce_scope_based_access: false}",
    None
)]
#[case::field_scope_mappings_empty_not_enforced(
    "{scope_mappings: [], enforce_scope_based_access: false}",
    None
)]
#[case::field_scope_mappings_enforcement_absent("{scope_mappings: []}", None)]
#[case::field_scope_mappings_enforcement_null(
    "{scope_mappings: [], enforce_scope_based_access: null}",
    None
)]
#[case::field_scope_mappings_null("{scope_mappings: null}", Some(false))]
#[case::field_enforce_scope_based_access_reference_text(
    "{scope_mappings: [], enforce_scope_based_access: os.environ/NO}",
    Some(false)
)]
#[case::field_enforce_scope_based_access_reference_true(
    "{scope_mappings: [], enforce_scope_based_access: os.environ/TRUE}",
    Some(true)
)]
#[case::field_enforce_scope_based_access_reference_false(
    "{scope_mappings: [], enforce_scope_based_access: os.environ/FALSE}",
    None
)]
#[case::field_enforce_scope_based_access_reference_padded_false(
    "{scope_mappings: [], enforce_scope_based_access: os.environ/PADDED}",
    None
)]
#[case::field_enforce_scope_based_access_reference_empty(
    "{scope_mappings: [], enforce_scope_based_access: os.environ/EMPTY}",
    None
)]
#[case::field_enforce_scope_based_access_reference_unset(
    "{scope_mappings: [], enforce_scope_based_access: os.environ/UNSET}",
    None
)]
fn scope_precheck_rows(#[case] yaml: &str, #[case] enforced: Option<bool>) {
    let environment = [
        ("NO", "no"),
        ("TRUE", "true"),
        ("FALSE", "false"),
        ("PADDED", " FALSE "),
        ("EMPTY", ""),
    ];
    let outcome = build(yaml, &environment)
        .map(|built| built.scopes.enforce_scope_based_access)
        .map_err(|error| error.to_string());
    assert_eq!(outcome, enforced.ok_or_else(|| SCOPES.to_owned()));
}

#[test]
fn scope_precheck_reads_plain_text_before_its_type() {
    let plain = "{scope_mappings: [], enforce_scope_based_access: 'false'}";
    assert_eq!(
        rejection(plain, &[]),
        "invalid value at enforce_scope_based_access"
    );
    let empty = "{scope_mappings: [], enforce_scope_based_access: ''}";
    assert_eq!(rejection(empty, &[]), SCOPES);
    let listed = "{scope_mappings: [], enforce_scope_based_access: [true]}";
    assert_eq!(
        rejection(listed, &[]),
        "invalid value at enforce_scope_based_access"
    );
}

#[test]
fn fallback_without_user_upsert_is_accepted() {
    let built = build("{fallback_to_db_teams: true, user_id_upsert: false}", &[]).unwrap();
    assert!(built.routing.fallback_to_db_teams);
    assert!(!built.claims.user_id_upsert);
}

#[test]
fn unknown_keys_list_the_allowed_arguments() {
    let message = rejection("{zeta: 1, alpha: null, admin_jwt_scope: scope}", &[]);
    assert_eq!(
        message,
        format!(
            "Invalid arguments provided: alpha, zeta. Allowed arguments are: {}.",
            ALLOWED_KEYS.join(", ")
        )
    );
    assert!(message.contains("admin_jwt_scope, admin_allowed_routes, team_id_jwt_field"));
    assert!(message.ends_with("fallback_to_db_teams, issuers."));
    build("{jwt_client_id_field: sub}", &[]).unwrap();
}

#[rstest]
#[case::unknown_key_before_paired_roles(
    "{user_roles_jwt_field: roles, extra: 1}",
    "Invalid arguments provided: extra."
)]
#[case::paired_roles_before_object_id(
    "{user_roles_jwt_field: roles, object_id_jwt_field: oid}",
    ROLES_PAIR
)]
#[case::object_id_before_scopes("{object_id_jwt_field: oid, scope_mappings: []}", OBJECT_ID)]
#[case::scopes_before_types("{scope_mappings: [], team_id_jwt_field: 5}", SCOPES)]
#[case::paired_roles_before_types("{user_roles_jwt_field: 5, admin_jwt_scope: 6}", ROLES_PAIR)]
#[case::first_field_in_annotation_order(
    "{issuers: 5, team_id_jwt_field: 5, admin_allowed_routes: 5}",
    "invalid value at admin_allowed_routes"
)]
#[case::annotation_order_not_input_order(
    "{fallback_to_db_teams: 5, public_key_ttl: x}",
    "invalid value at public_key_ttl"
)]
#[case::alias_type_in_its_slot(
    "{jwt_client_id_field: 5, virtual_key_mapping_cache_ttl: x}",
    "invalid value at virtual_key_claim_field"
)]
#[case::ttl_before_behavior(
    "{virtual_key_mapping_cache_ttl: x, unregistered_jwt_client_behavior: deny}",
    "invalid value at virtual_key_mapping_cache_ttl"
)]
#[case::record_field_before_extra(
    "{scope_mappings: [{scope: 5, extra: 1}], enforce_scope_based_access: true}",
    "invalid value at scope_mappings.0.scope"
)]
#[case::record_extra_before_audience(
    "{issuers: [{issuer: a, extra: 1}]}",
    "invalid value at issuers.0.extra"
)]
#[case::earlier_item_first(
    "{issuers: [{issuer: a}, {issuer: 5}]}",
    "JWT issuer a must configure audience or set disable_audience_validation=True"
)]
#[case::models_before_scope(
    "{scope_mappings: [{models: 5}], enforce_scope_based_access: true}",
    "invalid value at scope_mappings.0.models"
)]
fn constructor_validation_order(#[case] yaml: &str, #[case] expected: &str) {
    assert!(
        rejection(yaml, &[]).starts_with(expected),
        "{}",
        rejection(yaml, &[])
    );
}

#[rstest]
#[case::alias_alone("{jwt_client_id_field: sub}", Some("sub"))]
#[case::alias_reference("{jwt_client_id_field: os.environ/CLAIM}", Some("email"))]
#[case::alias_null("{jwt_client_id_field: null}", None)]
#[case::new_field_wins(
    "{jwt_client_id_field: sub, virtual_key_claim_field: email}",
    Some("email")
)]
#[case::null_new_field_wins("{jwt_client_id_field: sub, virtual_key_claim_field: null}", None)]
#[case::empty_new_field_wins("{jwt_client_id_field: sub, virtual_key_claim_field: ''}", Some(""))]
#[case::invalid_alias_dropped(
    "{jwt_client_id_field: 5, virtual_key_claim_field: email}",
    Some("email")
)]
#[case::neither("{}", None)]
fn aliases_presence_precedence(#[case] yaml: &str, #[case] expected: Option<&str>) {
    let built = build(yaml, &[("CLAIM", "email")]).unwrap();
    assert_eq!(
        built.virtual_keys.virtual_key_claim_field.as_deref(),
        expected
    );
    let dumped = super::dump(&built);
    assert!(!dumped.contains_key("jwt_client_id_field"));
    assert_eq!(
        dumped["virtual_key_claim_field"],
        expected.map_or(JsonData::Null, text)
    );
}

#![cfg(test)]

use betterlite_config::sections::auth_user_lookup::RbacRole;
use betterlite_config::sections::jwt_settings::Settings;
use betterlite_config::sections::jwt_settings::records::{RoleMap, RoleMapping, UserRole};
use betterlite_paths::error::PathError;
use rstest::rstest;

use super::error::RoleClaimError;
use super::fnmatch::RolePattern;
use super::rbac::{mapped_role, rbac_role};
use super::role_map::litellm_role;
use super::roles::{RoleField, is_allowed_user_role, role_claim};
use super::scopes::scopes;
use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::JwtClaims;
use crate::jwt_claims::settings::JwtAuthSettings;

fn document(raw: &str) -> TypedValue {
    serde_json::from_str(raw).unwrap()
}

fn documents(raw: &str) -> Vec<TypedValue> {
    serde_json::from_str(raw).unwrap()
}

fn identity<'auth>(raw: &str, settings: &'auth JwtAuthSettings) -> JwtClaims<'auth> {
    JwtClaims {
        claims: document(raw),
        settings,
        trusted_issuers: &[],
    }
}

fn identity_settings(team_field: Option<&str>, user_field: Option<&str>) -> JwtAuthSettings {
    JwtAuthSettings {
        team_id_jwt_field: team_field.map(str::to_owned),
        team_ids_jwt_field: None,
        team_id_default: None,
        team_alias_jwt_field: None,
        org_id_jwt_field: None,
        org_alias_jwt_field: None,
        user_id_jwt_field: user_field.map(str::to_owned),
        user_email_jwt_field: None,
        user_allowed_email_domain: None,
        user_id_upsert: false,
        end_user_id_jwt_field: None,
        agent_id_jwt_field: None,
        object_id_jwt_field: None,
    }
}

fn names(roles: &[&str]) -> Vec<String> {
    roles.iter().map(|role| (*role).to_owned()).collect()
}

fn user_role_settings(allowed: Option<&[&str]>) -> Settings {
    let mut settings = Settings::default();
    settings.claims.user_roles_jwt_field = Some("realm.roles".to_owned());
    settings.claims.user_allowed_roles = allowed.map(names);
    settings
}

fn role_settings(mappings: &[(&str, RbacRole)], role_map: &[(&str, UserRole)]) -> Settings {
    let mut settings = Settings::default();
    settings.rbac.roles_jwt_field = Some("roles".to_owned());
    let listed = mappings.iter().map(|(role, internal_role)| RoleMapping {
        role: (*role).to_owned(),
        internal_role: *internal_role,
    });
    settings.rbac.role_mappings = Some(listed.collect());
    let patterns = role_map.iter().map(|(jwt_role, litellm_role)| RoleMap {
        jwt_role: (*jwt_role).to_owned(),
        litellm_role: *litellm_role,
    });
    settings.sync.jwt_litellm_role_map = Some(patterns.collect());
    settings
}

fn rbac(raw: &str, identity_fields: &JwtAuthSettings, settings: &Settings) -> Option<RbacRole> {
    rbac_role(&identity(raw, identity_fields), settings).unwrap()
}

#[rstest]
#[case::empty_token("{}")]
#[case::other_claims(r#"{"sub":"user-1","scp":"litellm_proxy_admin"}"#)]
fn scope_missing_empty(#[case] raw: &str) {
    assert_eq!(scopes(&document(raw)).unwrap(), Vec::<TypedValue>::new());
}

#[rstest]
#[case::spaces(r#""read write""#, r#"["read","write"]"#)]
#[case::runs_and_edges(r#"" read\t\twrite\n admin ""#, r#"["read","write","admin"]"#)]
#[case::unicode_spaces(
    r#""read\u3000write\u00a0admin\u2028x""#,
    r#"["read","write","admin","x"]"#
)]
#[case::ascii_separators(r#""read\u001cwrite\u001fadmin""#, r#"["read","write","admin"]"#)]
#[case::zero_width_is_text(r#""read\u200bwrite""#, r#"["read\u200bwrite"]"#)]
#[case::empty(r#""""#, "[]")]
#[case::only_spaces(r#""  \t ""#, "[]")]
fn scope_whitespace_split(#[case] scope: &str, #[case] expected: &str) {
    let claims = document(&format!(r#"{{"scope":{scope}}}"#));
    assert_eq!(scopes(&claims).unwrap(), documents(expected));
}

#[test]
fn scope_list_preserves_shapes() {
    let listed = r#"["read write", 1, null, true, ["x"], {"k": "v"}, "litellm_proxy_admin"]"#;
    let claims = document(&format!(r#"{{"scope":{listed}}}"#));
    assert_eq!(scopes(&claims).unwrap(), documents(listed));
    let fields = identity_settings(None, None);
    let raw = r#"{"scope":["litellm_proxy_admin read"]}"#;
    assert_eq!(rbac(raw, &fields, &Settings::default()), None);
}

#[rstest]
#[case::null("null", "null")]
#[case::number("7", "a number")]
#[case::boolean("true", "a boolean")]
#[case::object(r#"{"read":true}"#, "an object")]
fn scope_other_types_error_before_identity(#[case] scope: &str, #[case] expected: &str) {
    let raw = format!(r#"{{"scope":{scope},"team":"team-1","sub":"user-1","roles":["admin"]}}"#);
    let error = scopes(&document(&raw)).unwrap_err();
    assert!(matches!(error, RoleClaimError::ScopeType { found } if found == expected));
    let fields = identity_settings(Some("team"), Some("sub"));
    let settings = role_settings(&[("admin", RbacRole::ProxyAdmin)], &[]);
    let outcome = rbac_role(&identity(&raw, &fields), &settings);
    assert!(matches!(outcome, Err(RoleClaimError::ScopeType { .. })));
}

#[rstest]
#[case::default_scope_in_string("read litellm_proxy_admin", None, Some(RbacRole::ProxyAdmin))]
#[case::longer_scope("litellm_proxy_admin_x", None, None)]
#[case::other_case("LITELLM_PROXY_ADMIN", None, None)]
#[case::configured_scope("read ops.admin", Some("ops.admin"), Some(RbacRole::ProxyAdmin))]
#[case::default_scope_when_configured("litellm_proxy_admin", Some("ops.admin"), None)]
fn admin_scope_exact(
    #[case] scope: &str,
    #[case] configured: Option<&str>,
    #[case] expected: Option<RbacRole>,
) {
    let mut settings = Settings::default();
    if let Some(admin) = configured {
        admin.clone_into(&mut settings.routes.admin_jwt_scope);
    }
    let fields = identity_settings(None, None);
    let raw = format!(r#"{{"scope":"{scope}"}}"#);
    assert_eq!(rbac(&raw, &fields, &settings), expected);
}

#[rstest]
#[case::unset_none(None, r#"{"realm":{"roles":["admin"]}}"#, None, None)]
#[case::unset_list(None, r#"{"realm":{"roles":["admin"]}}"#, Some(&["fallback"][..]), Some(r#"["fallback"]"#))]
#[case::missing_none(Some("realm.roles"), r#"{"realm":{}}"#, None, None)]
#[case::missing_list(Some("realm.roles"), r#"{"realm":{}}"#, Some(&["fallback"][..]), Some(r#"["fallback"]"#))]
#[case::through_a_list(Some("realm.roles"), r#"{"realm":["roles"]}"#, Some(&[][..]), Some("[]"))]
#[case::empty_path(Some(""), r#"{"":["admin"]}"#, Some(&["fallback"][..]), Some(r#"["fallback"]"#))]
#[case::found(Some("realm.roles"), r#"{"realm":{"roles":["admin"]}}"#, Some(&["fallback"][..]), Some(r#"["admin"]"#))]
fn roles_unset_or_missing_default(
    #[case] field: Option<&str>,
    #[case] raw: &str,
    #[case] default: Option<&[&str]>,
    #[case] expected: Option<&str>,
) {
    let mut settings = Settings::default();
    settings.claims.user_roles_jwt_field = field.map(str::to_owned);
    settings.rbac.roles_jwt_field = field.map(str::to_owned);
    let fallback = default.map(names);
    let wanted = expected.map(document);
    let claims = document(raw);
    let user_roles = role_claim(
        &claims,
        &settings,
        RoleField::UserRoles,
        fallback.as_deref(),
    );
    assert_eq!(user_roles.unwrap(), wanted);
    let roles = role_claim(&claims, &settings, RoleField::Roles, fallback.as_deref());
    assert_eq!(roles.unwrap(), wanted);
}

#[rstest]
#[case::null_without_default("null", None, None)]
#[case::string_not_wrapped(r#""admin""#, None, Some(r#""admin""#))]
#[case::object_kept(r#"{"admin":1}"#, None, Some(r#"{"admin":1}"#))]
#[case::mixed_list_kept(r#"["admin",1,null]"#, None, Some(r#"["admin",1,null]"#))]
#[case::null_takes_list_default("null", Some(&["fallback"][..]), Some(r#"["fallback"]"#))]
#[case::string_takes_list_default(r#""admin""#, Some(&["fallback"][..]), Some(r#"["fallback"]"#))]
fn roles_null_retained(
    #[case] claim: &str,
    #[case] default: Option<&[&str]>,
    #[case] expected: Option<&str>,
) {
    let settings = user_role_settings(Some(&["admin"]));
    let claims = document(&format!(r#"{{"realm":{{"roles":{claim}}}}}"#));
    let fallback = default.map(names);
    let found = role_claim(
        &claims,
        &settings,
        RoleField::UserRoles,
        fallback.as_deref(),
    );
    assert_eq!(found.unwrap(), expected.map(document));
}

#[test]
fn roles_dot_path_errors() {
    let mut settings = Settings::default();
    let claims = document(r#"{"kubernetes.io":{"roles":["ops"]},"roles":["top"]}"#);
    settings.rbac.roles_jwt_field = Some("kubernetes\\.io.roles".to_owned());
    let escaped = role_claim(&claims, &settings, RoleField::Roles, None).unwrap();
    assert_eq!(escaped, Some(document(r#"["ops"]"#)));
    settings.rbac.roles_jwt_field = Some("metadata.roles".to_owned());
    let unprefixed = role_claim(&claims, &settings, RoleField::Roles, None).unwrap();
    assert_eq!(unprefixed, Some(document(r#"["top"]"#)));
    settings.rbac.roles_jwt_field = Some("a".repeat(8193));
    let error = role_claim(&claims, &settings, RoleField::Roles, Some(&[])).unwrap_err();
    let expected = PathError::TooLong {
        bytes: 8193,
        limit: 8192,
    };
    assert!(matches!(error, RoleClaimError::Path { source, .. } if source == expected));
    settings.rbac.roles_jwt_field = Some("a".repeat(8192));
    assert_eq!(
        role_claim(&claims, &settings, RoleField::Roles, None).unwrap(),
        None
    );
    settings.rbac.roles_jwt_field = Some(vec!["a"; 257].join("."));
    let deep = role_claim(&claims, &settings, RoleField::Roles, None).unwrap_err();
    assert!(matches!(
        deep,
        RoleClaimError::Path {
            source: PathError::TooDeep { limit: 256 },
            ..
        }
    ));
}

#[rstest]
#[case::one_allowed(Some(r#"["dev","ops"]"#), Some(&["admin", "ops"][..]), true)]
#[case::other_case(Some(r#"["OPS"]"#), Some(&["admin", "ops"][..]), false)]
#[case::prefix(Some(r#"["op"]"#), Some(&["admin", "ops"][..]), false)]
#[case::number_member(Some("[1]"), Some(&["1"][..]), false)]
#[case::empty_claim(Some("[]"), Some(&["admin"][..]), false)]
#[case::null_claim(Some("null"), Some(&["admin"][..]), false)]
#[case::no_claim(None, Some(&["admin"][..]), false)]
#[case::no_allowed_roles(Some(r#"["admin"]"#), None, false)]
fn user_role_any_exact_member(
    #[case] claim: Option<&str>,
    #[case] allowed: Option<&[&str]>,
    #[case] expected: bool,
) {
    let settings = user_role_settings(allowed);
    let user_roles = claim.map(document);
    assert_eq!(
        is_allowed_user_role(user_roles.as_ref(), &settings).unwrap(),
        expected
    );
}

#[test]
fn role_string_character_semantics() {
    let letters = document(r#""ops""#);
    let allowed_letter = user_role_settings(Some(&["o"]));
    assert!(is_allowed_user_role(Some(&letters), &allowed_letter).unwrap());
    let allowed_word = user_role_settings(Some(&["ops"]));
    assert!(!is_allowed_user_role(Some(&letters), &allowed_word).unwrap());
    let fields = identity_settings(None, None);
    let raw = r#"{"realm":{"roles":"ops"}}"#;
    assert_eq!(
        rbac(raw, &fields, &allowed_letter),
        Some(RbacRole::InternalUser)
    );
    assert_eq!(rbac(raw, &fields, &allowed_word), None);
    let by_letter = role_settings(&[("ab", RbacRole::ProxyAdmin), ("b", RbacRole::Team)], &[]);
    assert_eq!(
        mapped_role(&document(r#"{"roles":"ab"}"#), &by_letter).unwrap(),
        Some(RbacRole::Team)
    );
}

#[test]
fn role_dict_key_semantics() {
    let keyed = document(r#"{"ops":false,"dev":null}"#);
    assert!(is_allowed_user_role(Some(&keyed), &user_role_settings(Some(&["ops"]))).unwrap());
    assert!(!is_allowed_user_role(Some(&keyed), &user_role_settings(Some(&["false"]))).unwrap());
    let settings = role_settings(
        &[("admin", RbacRole::ProxyAdmin), ("dev", RbacRole::Team)],
        &[],
    );
    let claims = document(r#"{"roles":{"dev":{"admin":true}}}"#);
    assert_eq!(
        mapped_role(&claims, &settings).unwrap(),
        Some(RbacRole::Team)
    );
}

#[rstest]
#[case::number("5", "a number")]
#[case::boolean("true", "a boolean")]
fn role_noniterable_and_unhashable_errors(#[case] claim: &str, #[case] expected: &str) {
    let value = document(claim);
    let allowed = user_role_settings(Some(&[]));
    let error = is_allowed_user_role(Some(&value), &allowed).unwrap_err();
    assert!(matches!(error, RoleClaimError::NotIterable { found } if found == expected));
    assert!(!is_allowed_user_role(Some(&value), &user_role_settings(None)).unwrap());
    let fields = identity_settings(None, None);
    let raw = format!(r#"{{"realm":{{"roles":{claim}}},"roles":["admin"]}}"#);
    let outcome = rbac_role(&identity(&raw, &fields), &allowed);
    assert!(matches!(outcome, Err(RoleClaimError::NotIterable { .. })));
    let settings = role_settings(&[("admin", RbacRole::ProxyAdmin)], &[]);
    let mapped = mapped_role(&document(&format!(r#"{{"roles":{claim}}}"#)), &settings);
    assert!(matches!(mapped, Err(RoleClaimError::NotIterable { .. })));
    let nested = mapped_role(&document(r#"{"roles":["admin",["dev"]]}"#), &settings);
    assert!(matches!(nested, Err(RoleClaimError::Unhashable)));
    let object = mapped_role(&document(r#"{"roles":["admin",{"dev":1}]}"#), &settings);
    assert!(matches!(object, Err(RoleClaimError::Unhashable)));
    let scalars = mapped_role(&document(r#"{"roles":[1,null,true,"admin"]}"#), &settings);
    assert_eq!(scalars.unwrap(), Some(RbacRole::ProxyAdmin));
}

#[rstest]
#[case::admin_over_team(
    r#"{"scope":"litellm_proxy_admin","team":"t","sub":"u","roles":["dev"]}"#,
    Some(RbacRole::ProxyAdmin)
)]
#[case::team_over_user(r#"{"team":"t","sub":"u","roles":["dev"]}"#, Some(RbacRole::Team))]
#[case::user_over_mapping(r#"{"sub":"u","roles":["dev"]}"#, Some(RbacRole::InternalUser))]
#[case::allowed_role_over_mapping(
    r#"{"realm":{"roles":["ops"]},"roles":["dev"]}"#,
    Some(RbacRole::InternalUser)
)]
#[case::mapping_last(
    r#"{"realm":{"roles":["guest"]},"roles":["dev"]}"#,
    Some(RbacRole::ProxyAdmin)
)]
#[case::nothing(r#"{"realm":{"roles":["guest"]},"roles":["guest"]}"#, None)]
fn rbac_admin_team_user_mapping_precedence(#[case] raw: &str, #[case] expected: Option<RbacRole>) {
    let fields = identity_settings(Some("team"), Some("sub"));
    let mut settings = role_settings(&[("dev", RbacRole::ProxyAdmin)], &[]);
    settings.claims.user_roles_jwt_field = Some("realm.roles".to_owned());
    settings.claims.user_allowed_roles = Some(names(&["ops"]));
    assert_eq!(rbac(raw, &fields, &settings), expected);
}

#[test]
fn nonnull_empty_identity_counts() {
    let both = identity_settings(Some("team"), Some("sub"));
    let settings = Settings::default();
    assert_eq!(
        rbac(r#"{"team":"","sub":"u"}"#, &both, &settings),
        Some(RbacRole::Team)
    );
    assert_eq!(
        rbac(r#"{"team":null,"sub":""}"#, &both, &settings),
        Some(RbacRole::InternalUser)
    );
    assert_eq!(rbac(r#"{"team":null,"sub":null}"#, &both, &settings), None);
    let mut defaulted = identity_settings(None, None);
    defaulted.team_id_default = Some(String::new());
    assert_eq!(rbac("{}", &defaulted, &settings), Some(RbacRole::Team));
}

#[test]
fn user_identity_bypasses_allowed_roles() {
    let fields = identity_settings(None, Some("sub"));
    let settings = user_role_settings(Some(&["ops"]));
    let unlisted = r#"{"sub":"u","realm":{"roles":["guest"]}}"#;
    assert_eq!(
        rbac(unlisted, &fields, &settings),
        Some(RbacRole::InternalUser)
    );
    let noniterable = r#"{"sub":"u","realm":{"roles":5}}"#;
    assert_eq!(
        rbac(noniterable, &fields, &settings),
        Some(RbacRole::InternalUser)
    );
}

#[test]
fn role_mapping_order_and_exactness() {
    let settings = role_settings(
        &[
            ("dev", RbacRole::InternalUser),
            ("admin", RbacRole::ProxyAdmin),
            ("adm*", RbacRole::Team),
        ],
        &[],
    );
    let both = document(r#"{"roles":["admin","dev"]}"#);
    assert_eq!(
        mapped_role(&both, &settings).unwrap(),
        Some(RbacRole::InternalUser)
    );
    let admin = document(r#"{"roles":["admin"]}"#);
    assert_eq!(
        mapped_role(&admin, &settings).unwrap(),
        Some(RbacRole::ProxyAdmin)
    );
    let wildcard = document(r#"{"roles":["administrator","Admin"]}"#);
    assert_eq!(mapped_role(&wildcard, &settings).unwrap(), None);
    let literal = document(r#"{"roles":["adm*"]}"#);
    assert_eq!(
        mapped_role(&literal, &settings).unwrap(),
        Some(RbacRole::Team)
    );
}

#[test]
fn litellm_role_map_configuration_order() {
    let settings = role_settings(
        &[],
        &[
            ("viewer*", UserRole::ProxyAdminViewer),
            ("*", UserRole::InternalUser),
        ],
    );
    let later_role = document(r#"{"roles":["dev","viewer-1"]}"#);
    assert_eq!(
        litellm_role(&later_role, &settings).unwrap(),
        Some(UserRole::ProxyAdminViewer)
    );
    let other = document(r#"{"roles":["dev"]}"#);
    assert_eq!(
        litellm_role(&other, &settings).unwrap(),
        Some(UserRole::InternalUser)
    );
    let matched_first = document(r#"{"roles":["viewer",5]}"#);
    assert_eq!(
        litellm_role(&matched_first, &settings).unwrap(),
        Some(UserRole::ProxyAdminViewer)
    );
    let number_first = litellm_role(&document(r#"{"roles":[5,"viewer"]}"#), &settings);
    assert!(matches!(
        number_first,
        Err(RoleClaimError::NotText { found: "a number" })
    ));
    let reached_later = litellm_role(&document(r#"{"roles":["dev",null]}"#), &settings);
    assert!(matches!(
        reached_later,
        Err(RoleClaimError::NotText { found: "null" })
    ));
    let string_claim = document(r#"{"roles":"viewer"}"#);
    assert_eq!(litellm_role(&string_claim, &settings).unwrap(), None);
}

#[rstest]
#[case::star("a*", "abc", true)]
#[case::case_sensitive("A*", "abc", false)]
#[case::question("a?c", "abc", true)]
#[case::question_takes_slash("a?c", "a/c", true)]
#[case::star_takes_slash("*", "a/b", true)]
#[case::question_takes_newline("?", "\n", true)]
#[case::set("[abc]", "b", true)]
#[case::negated_set("[!abc]", "b", false)]
#[case::range("[a-c]x", "bx", true)]
#[case::reversed_range_empty("[z-a]", "a", false)]
#[case::single_letter_range("[a-a]", "a", true)]
#[case::negated_reversed_range_any("[!z-a]", "q", true)]
#[case::fold_brings_negation("[z-a!]", "q", true)]
#[case::folded_negation_keeps_join_hyphen("[b-a!-z]", "-", false)]
#[case::folded_negation_rejects_member("[b-a!-z]", "z", false)]
#[case::folded_negation_accepts_other("[b-a!-z]", "q", true)]
#[case::reversed_range_rejects_hyphen("[z-a]", "-", false)]
#[case::negated_reversed_range_takes_hyphen("[!z-a]", "-", true)]
#[case::two_ranges_reject_hyphen("[a-cx-z]", "-", false)]
#[case::unclosed_bracket_literal("[", "[", true)]
#[case::unclosed_bracket_inside("a[b", "a[b", true)]
#[case::leading_close_bracket("[]]", "]", true)]
#[case::negated_close_bracket("[!]]", "a", true)]
#[case::negated_close_bracket_rejects("[!]]", "]", false)]
#[case::negated_close_bracket_is_one_letter("[!]]", "a]", false)]
#[case::hyphen_after_negation_is_a_member("[!-a]", "-", false)]
#[case::trailing_hyphen("[a-]", "-", true)]
#[case::hyphen_after_range("[a-c-e]", "-", true)]
#[case::hyphen_after_range_no_span("[a-c-e]", "d", false)]
#[case::caret_is_literal("[^a]", "^", true)]
#[case::caret_does_not_negate("[^a]", "b", false)]
#[case::backslash_literal("[\\]", "\\", true)]
#[case::runs_backtrack("*a*b", "xaxb", true)]
#[case::runs_anchor_end("*a*b", "xaxbc", false)]
#[case::empty_pattern("", "", true)]
#[case::empty_pattern_rejects("", "a", false)]
#[case::double_star_empty("**", "", true)]
fn fnmatch_case_slash_question_brackets(
    #[case] pattern: &str,
    #[case] name: &str,
    #[case] expected: bool,
) {
    assert_eq!(RolePattern::new(pattern).matches(name), expected);
}

#[test]
fn empty_maps_and_roles_none() {
    let roles =
        document(r#"{"roles":["admin"],"empty":[],"blank":"","zero":0,"off":false,"void":{}}"#);
    let mut unset = role_settings(&[], &[]);
    assert_eq!(litellm_role(&roles, &unset).unwrap(), None);
    unset.sync.jwt_litellm_role_map = None;
    assert_eq!(litellm_role(&roles, &unset).unwrap(), None);
    let mut matching = role_settings(&[], &[("*", UserRole::InternalUser)]);
    matching.rbac.roles_jwt_field = Some("empty".to_owned());
    assert_eq!(litellm_role(&roles, &matching).unwrap(), None);
    matching.rbac.roles_jwt_field = None;
    assert_eq!(litellm_role(&roles, &matching).unwrap(), None);
    let mut mappings = role_settings(&[], &[]);
    assert_eq!(mapped_role(&roles, &mappings).unwrap(), None);
    mappings = role_settings(&[("admin", RbacRole::ProxyAdmin)], &[]);
    mappings.rbac.role_mappings = None;
    assert_eq!(mapped_role(&roles, &mappings).unwrap(), None);
}

#[rstest]
#[case::empty_list("empty")]
#[case::empty_string("blank")]
#[case::zero("zero")]
#[case::false_claim("off")]
#[case::empty_object("void")]
#[case::missing("absent")]
fn empty_role_claims_map_to_none(#[case] field: &str) {
    let claims = document(r#"{"empty":[],"blank":"","zero":0.0,"off":false,"void":{}}"#);
    let mut settings = role_settings(&[("", RbacRole::ProxyAdmin)], &[]);
    settings.rbac.roles_jwt_field = Some(field.to_owned());
    assert_eq!(mapped_role(&claims, &settings).unwrap(), None);
}

#[test]
fn no_mapping_none() {
    let claims = document(r#"{"roles":["guest","viewer"]}"#);
    let mappings = role_settings(&[("admin", RbacRole::ProxyAdmin)], &[]);
    assert_eq!(mapped_role(&claims, &mappings).unwrap(), None);
    let fields = identity_settings(Some("team"), Some("sub"));
    assert_eq!(
        rbac(r#"{"roles":["guest","viewer"]}"#, &fields, &mappings),
        None
    );
    let role_map = role_settings(
        &[],
        &[
            ("admin*", UserRole::ProxyAdmin),
            ("ops", UserRole::OrgAdmin),
        ],
    );
    assert_eq!(litellm_role(&claims, &role_map).unwrap(), None);
}

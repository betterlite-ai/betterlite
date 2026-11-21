use rstest::rstest;

use super::super::records::{
    IssuerConfig, OverridePath, RoleMap, RoleMapping, RoutingOverride, ScopeMapping, Selector,
    UnregisteredClientBehavior, UserRole,
};
use super::{build, rejection};
use crate::sections::auth_user_lookup::RbacRole;

const ISSUER_FIELDS: &str = "jwks_url: https://idp/jwks, user_id_jwt_field: sub, user_email_jwt_field: email, team_id_jwt_field: team, team_ids_jwt_field: teams, org_id_jwt_field: org, end_user_id_jwt_field: eid, virtual_key_claim_field: client, unregistered_jwt_client_behavior: reject";

#[rstest]
#[case::field_role_mappings_valid(
    "{role_mappings: [{role: admin, internal_role: proxy_admin}, {role: dev, internal_role: internal_user}, {role: svc, internal_role: team}]}",
    ""
)]
#[case::field_role_mappings_empty("{role_mappings: []}", "")]
#[case::field_role_mappings_extra_ignored(
    "{role_mappings: [{role: admin, internal_role: proxy_admin, note: x}]}",
    ""
)]
#[case::field_role_mappings_missing_role(
    "{role_mappings: [{internal_role: proxy_admin}]}",
    "role_mappings.0.role"
)]
#[case::field_role_mappings_null_role(
    "{role_mappings: [{role: null, internal_role: proxy_admin}]}",
    "role_mappings.0.role"
)]
#[case::field_role_mappings_number_role(
    "{role_mappings: [{role: 5, internal_role: proxy_admin}]}",
    "role_mappings.0.role"
)]
#[case::field_role_mappings_missing_internal_role(
    "{role_mappings: [{role: admin}]}",
    "role_mappings.0.internal_role"
)]
#[case::field_role_mappings_non_rbac_role(
    "{role_mappings: [{role: admin, internal_role: org_admin}]}",
    "role_mappings.0.internal_role"
)]
#[case::field_role_mappings_text_item("{role_mappings: [admin]}", "role_mappings.0")]
#[case::field_role_mappings_null_item(
    "{role_mappings: [{role: a, internal_role: team}, null]}",
    "role_mappings.1"
)]
#[case::field_role_mappings_sequence_item(
    "{role_mappings: [[admin, proxy_admin]]}",
    "role_mappings.0"
)]
#[case::field_role_mappings_not_a_list("{role_mappings: {role: admin}}", "role_mappings")]
fn role_mapping_rows(#[case] yaml: &str, #[case] rejected_at: &str) {
    if rejected_at.is_empty() {
        build(yaml, &[]).unwrap();
    } else {
        assert_eq!(
            rejection(yaml, &[]),
            format!("invalid value at {rejected_at}")
        );
    }
}

#[test]
fn role_mappings_keep_their_values() {
    let built = build(
        "{role_mappings: [{role: admin, internal_role: proxy_admin, note: x}]}",
        &[],
    )
    .unwrap();
    assert_eq!(
        built.rbac.role_mappings,
        Some(vec![RoleMapping {
            role: "admin".to_owned(),
            internal_role: RbacRole::ProxyAdmin
        }])
    );
}

#[rstest]
#[case::field_jwt_litellm_role_map_proxy_admin("proxy_admin", UserRole::ProxyAdmin)]
#[case::field_jwt_litellm_role_map_proxy_admin_viewer(
    "proxy_admin_viewer",
    UserRole::ProxyAdminViewer
)]
#[case::field_jwt_litellm_role_map_org_admin("org_admin", UserRole::OrgAdmin)]
#[case::field_jwt_litellm_role_map_internal_user("internal_user", UserRole::InternalUser)]
#[case::field_jwt_litellm_role_map_internal_user_viewer(
    "internal_user_viewer",
    UserRole::InternalUserViewer
)]
#[case::field_jwt_litellm_role_map_team("team", UserRole::Team)]
#[case::field_jwt_litellm_role_map_customer("customer", UserRole::Customer)]
fn role_map_roles(#[case] role: &str, #[case] expected: UserRole) {
    let yaml =
        format!("{{jwt_litellm_role_map: [{{jwt_role: group, litellm_role: {role}, weight: 1}}]}}");
    let built = build(&yaml, &[]).unwrap();
    assert_eq!(
        built.sync.jwt_litellm_role_map,
        Some(vec![RoleMap {
            jwt_role: "group".to_owned(),
            litellm_role: expected
        }])
    );
}

#[rstest]
#[case::field_jwt_litellm_role_map_unknown_role(
    "{jwt_litellm_role_map: [{jwt_role: g, litellm_role: admin}]}",
    "jwt_litellm_role_map.0.litellm_role"
)]
#[case::field_jwt_litellm_role_map_missing_jwt_role(
    "{jwt_litellm_role_map: [{litellm_role: team}]}",
    "jwt_litellm_role_map.0.jwt_role"
)]
#[case::field_jwt_litellm_role_map_number_jwt_role(
    "{jwt_litellm_role_map: [{jwt_role: 1, litellm_role: team}]}",
    "jwt_litellm_role_map.0.jwt_role"
)]
#[case::field_jwt_litellm_role_map_not_a_list(
    "{jwt_litellm_role_map: team}",
    "jwt_litellm_role_map"
)]
fn role_map_rejections(#[case] yaml: &str, #[case] rejected_at: &str) {
    assert_eq!(
        rejection(yaml, &[]),
        format!("invalid value at {rejected_at}")
    );
    assert_eq!(
        build("{jwt_litellm_role_map: null}", &[])
            .unwrap()
            .sync
            .jwt_litellm_role_map,
        None
    );
}

#[rstest]
#[case::field_scope_mappings_full("[{scope: read, models: [gpt], routes: [/chat]}]", ScopeMapping { scope: "read".to_owned(), models: Some(vec!["gpt".to_owned()]), routes: Some(vec!["/chat".to_owned()]) })]
#[case::field_scope_mappings_scope_only("[{scope: read}]", ScopeMapping { scope: "read".to_owned(), models: None, routes: None })]
#[case::field_scope_mappings_null_lists("[{scope: '', models: null, routes: []}]", ScopeMapping { scope: String::new(), models: None, routes: Some(Vec::new()) })]
fn scope_mapping_values(#[case] mappings: &str, #[case] expected: ScopeMapping) {
    let built = build(
        &format!("{{scope_mappings: {mappings}, enforce_scope_based_access: true}}"),
        &[],
    )
    .unwrap();
    assert_eq!(built.scopes.scope_mappings, Some(vec![expected]));
}

#[rstest]
#[case::field_scope_mappings_extra_forbidden("[{scope: read, role: x}]", "scope_mappings.0.role")]
#[case::field_scope_mappings_missing_scope("[{models: [gpt]}]", "scope_mappings.0.scope")]
#[case::field_scope_mappings_bad_route("[{scope: read, routes: [5]}]", "scope_mappings.0.routes.0")]
#[case::field_scope_mappings_text("read", "scope_mappings")]
fn scope_mapping_rejections(#[case] mappings: &str, #[case] rejected_at: &str) {
    let yaml = format!("{{scope_mappings: {mappings}, enforce_scope_based_access: true}}");
    assert_eq!(
        rejection(&yaml, &[]),
        format!("invalid value at {rejected_at}")
    );
}

#[rstest]
#[case::field_routing_overrides_issuer_only("{iss: a}", RoutingOverride { iss: Selector::One("a".to_owned()), client_id: None, scope: None, aud: None, path: OverridePath::Oauth2 })]
#[case::field_routing_overrides_lists("{iss: [a, b], client_id: [c], scope: s, aud: [], path: oauth2}", RoutingOverride { iss: Selector::Many(vec!["a".to_owned(), "b".to_owned()]), client_id: Some(Selector::Many(vec!["c".to_owned()])), scope: Some(Selector::One("s".to_owned())), aud: Some(Selector::Many(Vec::new())), path: OverridePath::Oauth2 })]
#[case::field_routing_overrides_empty_selectors("{iss: '', client_id: '', scope: null, aud: '*', path: null}", RoutingOverride { iss: Selector::One(String::new()), client_id: Some(Selector::One(String::new())), scope: None, aud: Some(Selector::One("*".to_owned())), path: OverridePath::Oauth2 })]
fn routing_override_values(#[case] rule: &str, #[case] expected: RoutingOverride) {
    let built = build(&format!("{{routing_overrides: [{rule}]}}"), &[]).unwrap();
    assert_eq!(built.routing.routing_overrides, Some(vec![expected]));
}

#[rstest]
#[case::field_routing_overrides_missing_iss("{client_id: c}", "routing_overrides.0.iss")]
#[case::field_routing_overrides_null_iss("{iss: null}", "routing_overrides.0.iss")]
#[case::field_routing_overrides_number_iss("{iss: 5}", "routing_overrides.0.iss")]
#[case::field_routing_overrides_mixed_list("{iss: a, aud: [a, 5]}", "routing_overrides.0.aud")]
#[case::field_routing_overrides_other_path("{iss: a, path: api_key}", "routing_overrides.0.path")]
#[case::field_routing_overrides_extra_forbidden(
    "{iss: a, match: all}",
    "routing_overrides.0.match"
)]
fn routing_override_rejections(#[case] rule: &str, #[case] rejected_at: &str) {
    let yaml = format!("{{routing_overrides: [{rule}]}}");
    assert_eq!(
        rejection(&yaml, &[]),
        format!("invalid value at {rejected_at}")
    );
}

#[test]
fn issuers_keep_every_field() {
    let yaml =
        format!("{{issuers: [{{issuer: https://idp, audience: [api, web], {ISSUER_FIELDS}}}]}}");
    let built = build(&yaml, &[]).unwrap();
    assert_eq!(
        built.routing.issuers,
        Some(vec![IssuerConfig {
            issuer: "https://idp".to_owned(),
            jwks_url: Some("https://idp/jwks".to_owned()),
            audience: Some(Selector::Many(vec!["api".to_owned(), "web".to_owned()])),
            disable_audience_validation: false,
            user_id_jwt_field: Some("sub".to_owned()),
            user_email_jwt_field: Some("email".to_owned()),
            team_id_jwt_field: Some("team".to_owned()),
            team_ids_jwt_field: Some("teams".to_owned()),
            org_id_jwt_field: Some("org".to_owned()),
            end_user_id_jwt_field: Some("eid".to_owned()),
            virtual_key_claim_field: Some("client".to_owned()),
            unregistered_client: Some(UnregisteredClientBehavior::Reject),
        }])
    );
}

#[rstest]
#[case::field_issuers_audience("{issuer: a, audience: api}", "")]
#[case::field_issuers_empty_audience("{issuer: a, audience: ''}", "")]
#[case::field_issuers_empty_audience_list("{issuer: a, audience: []}", "")]
#[case::field_issuers_disabled("{issuer: a, disable_audience_validation: true}", "")]
#[case::field_issuers_disabled_null_audience(
    "{issuer: a, audience: null, disable_audience_validation: true}",
    ""
)]
#[case::field_issuers_missing_audience(
    "{issuer: a}",
    "JWT issuer a must configure audience or set disable_audience_validation=True"
)]
#[case::field_issuers_null_flag(
    "{issuer: a, disable_audience_validation: null}",
    "JWT issuer a must configure audience or set disable_audience_validation=True"
)]
#[case::field_issuers_audience_and_disabled(
    "{issuer: b, audience: api, disable_audience_validation: true}",
    "JWT issuer b cannot set audience and disable_audience_validation=True together"
)]
#[case::field_issuers_empty_audience_and_disabled(
    "{issuer: b, audience: '', disable_audience_validation: true}",
    "JWT issuer b cannot set audience and disable_audience_validation=True together"
)]
#[case::field_issuers_missing_issuer("{audience: api}", "invalid value at issuers.0.issuer")]
#[case::field_issuers_number_audience(
    "{issuer: a, audience: 5}",
    "invalid value at issuers.0.audience"
)]
#[case::field_issuers_text_flag(
    "{issuer: a, audience: api, disable_audience_validation: 'no'}",
    "invalid value at issuers.0.disable_audience_validation"
)]
#[case::field_issuers_unknown_behavior(
    "{issuer: a, audience: api, unregistered_jwt_client_behavior: deny}",
    "invalid value at issuers.0.unregistered_jwt_client_behavior"
)]
#[case::field_issuers_extra_forbidden(
    "{issuer: a, audience: api, jwks: x}",
    "invalid value at issuers.0.jwks"
)]
#[case::field_issuers_reference_kept("{issuer: os.environ/ISSUER, audience: api}", "")]
fn issuer_rows(#[case] issuer: &str, #[case] expected: &str) {
    let yaml = format!("{{issuers: [{issuer}]}}");
    if expected.is_empty() {
        build(&yaml, &[("ISSUER", "resolved")]).unwrap();
    } else {
        assert_eq!(rejection(&yaml, &[]), expected);
    }
}

#[test]
fn nested_references_stay_text() {
    let yaml = "{issuers: [{issuer: os.environ/ISSUER, audience: os.environ/AUD}], role_mappings: [{role: os.environ/ROLE, internal_role: team}]}";
    let built = build(yaml, &[("ISSUER", "a"), ("AUD", "b"), ("ROLE", "c")]).unwrap();
    let issuers = built.routing.issuers.unwrap();
    assert_eq!(issuers[0].issuer, "os.environ/ISSUER");
    assert_eq!(
        issuers[0].audience,
        Some(Selector::One("os.environ/AUD".to_owned()))
    );
    assert_eq!(built.rbac.role_mappings.unwrap()[0].role, "os.environ/ROLE");
}

const ISSUERS: &str = "{virtual_key_claim_field: global, unregistered_jwt_client_behavior: auto_register, issuers: [{issuer: a, audience: x, virtual_key_claim_field: first, unregistered_jwt_client_behavior: reject}, {issuer: a, audience: y, virtual_key_claim_field: second}, {issuer: b, audience: z}, {issuer: 'a/', disable_audience_validation: true, virtual_key_claim_field: slash}]}";

#[rstest]
#[case::issuer_first_exact_match(
    Some("a"),
    Some("x"),
    Some("first"),
    UnregisteredClientBehavior::Reject
)]
#[case::issuer_null_overrides_fall_back(
    Some("b"),
    Some("z"),
    Some("global"),
    UnregisteredClientBehavior::AutoRegister
)]
#[case::issuer_no_normalization(
    Some("A"),
    None,
    Some("global"),
    UnregisteredClientBehavior::AutoRegister
)]
#[case::issuer_trailing_slash_is_its_own(
    Some("a/"),
    None,
    Some("slash"),
    UnregisteredClientBehavior::AutoRegister
)]
#[case::issuer_missing(None, None, Some("global"), UnregisteredClientBehavior::AutoRegister)]
fn issuer_accessors(
    #[case] issuer: Option<&str>,
    #[case] audience: Option<&str>,
    #[case] claim_field: Option<&str>,
    #[case] behavior: UnregisteredClientBehavior,
) {
    let built = build(ISSUERS, &[]).unwrap();
    let matched = built.issuer_config(issuer);
    assert_eq!(
        matched.map(|config| config.issuer.as_str()),
        issuer.filter(|name| *name != "A")
    );
    if let Some(expected) = audience {
        assert_eq!(
            matched.unwrap().audience,
            Some(Selector::One(expected.to_owned()))
        );
    }
    assert_eq!(built.claim_field(issuer), claim_field);
    assert_eq!(built.client_behavior(issuer), behavior);
    assert_eq!(built.routing.issuers.as_ref().map(Vec::len), Some(4));
}

#[rstest]
#[case::mapping_unset("{}", false)]
#[case::mapping_global_empty("{virtual_key_claim_field: ''}", true)]
#[case::mapping_issuer_empty(
    "{issuers: [{issuer: a, audience: x}, {issuer: b, audience: y, virtual_key_claim_field: ''}]}",
    true
)]
#[case::mapping_issuer_unset("{issuers: [{issuer: a, audience: x}]}", false)]
#[case::mapping_null_issuers("{issuers: null, virtual_key_claim_field: null}", false)]
fn virtual_key_mapping_configured(#[case] yaml: &str, #[case] configured: bool) {
    let built = build(yaml, &[]).unwrap();
    assert_eq!(built.mapping_configured(), configured);
}

#[test]
fn accessors_without_issuers_use_the_globals() {
    let built = build("{unregistered_jwt_client_behavior: reject}", &[]).unwrap();
    assert_eq!(built.issuer_config(Some("a")), None);
    assert_eq!(built.claim_field(Some("a")), None);
    assert_eq!(
        built.client_behavior(Some("a")),
        UnregisteredClientBehavior::Reject
    );
    let defaults = build("{}", &[]).unwrap();
    assert_eq!(
        defaults.client_behavior(None),
        UnregisteredClientBehavior::FallbackTeamMapping
    );
}

use crate::jwt_claims::claims::ClaimError;
use crate::jwt_claims::handler::JwtHandler;
use crate::jwt_claims::settings::JwtAuthSettings;
use crate::sso_claims::claims::SsoClaimsError;
use crate::sso_claims::convert::{ConversionConfig, convert_generic_response};
use crate::sso_types::identity::OpenIdIdentity;
use crate::sso_types::records::{RoleMappings, TeamMappings};
use crate::user_roles::UserRole;

use super::support::{attributes, claims, document, handler, mappings, strings, team_handler};

fn config<'config>(
    jwt_handler: &'config JwtHandler,
    sso_jwt_handler: Option<&'config JwtHandler>,
    role_mappings: Option<&'config RoleMappings>,
    team_mappings: Option<&'config TeamMappings>,
) -> ConversionConfig<'config> {
    ConversionConfig {
        jwt_handler,
        sso_jwt_handler,
        role_mappings,
        team_mappings,
    }
}

fn convert(
    response: &str,
    config: &ConversionConfig<'_>,
    bindings: &[(&str, &str)],
) -> Result<OpenIdIdentity, SsoClaimsError> {
    convert_generic_response(&document(response), config, &attributes(bindings))
}

fn plain(response: &str) -> OpenIdIdentity {
    let jwt_handler = handler(JwtAuthSettings::default());
    convert(response, &config(&jwt_handler, None, None, None), &[]).unwrap()
}

#[test]
fn fields_come_from_the_default_attribute_paths() {
    let converted = plain(
        r#"{"preferred_username":"user-1","sub":"subject-1","email":"User@Example.com","first_name":"First","last_name":"Last","provider":"generic"}"#,
    );
    assert_eq!(converted.id.as_deref(), Some("user-1"));
    assert_eq!(converted.display_name.as_deref(), Some("subject-1"));
    assert_eq!(converted.email.as_deref(), Some("user@example.com"));
    assert_eq!(converted.first_name.as_deref(), Some("First"));
    assert_eq!(converted.last_name.as_deref(), Some("Last"));
    assert_eq!(converted.provider.as_deref(), Some("generic"));
    assert!(converted.picture.is_none());
    assert!(converted.team_ids.is_empty());
    assert!(converted.user_role.is_none());
    assert!(converted.extra_fields.is_none());
}

#[test]
fn custom_attribute_bindings_are_followed() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let converted = convert(
        r#"{"uid":"user-2","profile":{"name":"Profile Name"},"contact":{"mail":"Nested@Example.com"},"given_name":"Given","family_name":"Family","idp":"okta","custom_roles":["internal_user","proxy_admin"]}"#,
        &config(&jwt_handler, None, None, None),
        &[
            ("GENERIC_USER_ID_ATTRIBUTE", "uid"),
            ("GENERIC_USER_DISPLAY_NAME_ATTRIBUTE", "profile.name"),
            ("GENERIC_USER_EMAIL_ATTRIBUTE", "contact.mail"),
            ("GENERIC_USER_FIRST_NAME_ATTRIBUTE", "given_name"),
            ("GENERIC_USER_LAST_NAME_ATTRIBUTE", "family_name"),
            ("GENERIC_USER_PROVIDER_ATTRIBUTE", "idp"),
            ("GENERIC_USER_ROLE_ATTRIBUTE", "custom_roles"),
        ],
    )
    .unwrap();
    assert_eq!(converted.id.as_deref(), Some("user-2"));
    assert_eq!(converted.display_name.as_deref(), Some("Profile Name"));
    assert_eq!(converted.email.as_deref(), Some("nested@example.com"));
    assert_eq!(converted.first_name.as_deref(), Some("Given"));
    assert_eq!(converted.last_name.as_deref(), Some("Family"));
    assert_eq!(converted.provider.as_deref(), Some("okta"));
    assert_eq!(converted.user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn an_email_is_lowercased_and_a_missing_one_stays_absent() {
    let converted = plain(
        r#"{"preferred_username":"user-1","sub":"subject-1","email":"User@Example.COM","first_name":"First","last_name":"Last","provider":"generic"}"#,
    );
    assert_eq!(converted.email.as_deref(), Some("user@example.com"));
    assert_eq!(plain(r#"{"email":""}"#).email.as_deref(), Some(""));
    assert!(plain(r#"{"preferred_username":"user-1"}"#).email.is_none());
}

#[test]
fn a_null_field_stays_absent() {
    let converted = plain(r#"{"preferred_username":null,"email":null}"#);
    assert!(converted.id.is_none());
    assert!(converted.email.is_none());
    assert!(converted.display_name.is_none());
}

#[test]
fn a_non_text_field_names_the_path() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let error = convert(
        r#"{"email":42}"#,
        &config(&jwt_handler, None, None, None),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(&error, SsoClaimsError::Claim(ClaimError::Type { claim, .. }) if claim == "email"),
        "{error:?}"
    );
}

#[test]
fn extra_fields_copy_each_configured_path() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let converted = convert(
        r#"{"tenant":{"id":"t-1"},"tier":"gold","kubernetes.io":{"namespace":"ns"},"absent":null}"#,
        &config(&jwt_handler, None, None, None),
        &[(
            "GENERIC_USER_EXTRA_ATTRIBUTES",
            "tenant.id, metadata.tier ,kubernetes\\.io.namespace,absent,",
        )],
    )
    .unwrap();
    assert_eq!(
        converted.extra_fields,
        Some(claims(
            r#"{"tenant.id":"t-1","metadata.tier":"gold","kubernetes\\.io.namespace":"ns","absent":null,"":null}"#
        ))
    );
}

#[test]
fn absent_extra_attributes_leave_extra_fields_unset() {
    let converted = plain(r#"{"tenant":{"id":"t-1"}}"#);
    assert!(converted.extra_fields.is_none());
}

#[test]
fn a_generic_mapping_decides_the_role() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let role_mappings = mappings(None, &[(UserRole::ProxyAdmin, &["admins"])]);
    let converted = convert(
        r#"{"groups":["admins"],"role":"internal_user"}"#,
        &config(&jwt_handler, None, Some(&role_mappings), None),
        &[],
    )
    .unwrap();
    assert_eq!(converted.user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn an_okta_mapping_ignores_the_provider_case() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let mut role_mappings = mappings(None, &[(UserRole::ProxyAdminViewer, &["viewers"])]);
    role_mappings.provider = "OKTA".to_owned();
    let converted = convert(
        r#"{"groups":["viewers"],"role":"internal_user"}"#,
        &config(&jwt_handler, None, Some(&role_mappings), None),
        &[],
    )
    .unwrap();
    assert_eq!(converted.user_role, Some(UserRole::ProxyAdminViewer));
}

#[test]
fn a_mapping_for_another_provider_is_ignored() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let mut role_mappings = mappings(None, &[(UserRole::ProxyAdmin, &["admins"])]);
    role_mappings.provider = "google".to_owned();
    let converted = convert(
        r#"{"groups":["admins"],"role":"internal_user"}"#,
        &config(&jwt_handler, None, Some(&role_mappings), None),
        &[],
    )
    .unwrap();
    assert_eq!(converted.user_role, Some(UserRole::InternalUser));
}

#[test]
fn the_default_role_stands_in_without_a_group_claim() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let role_mappings = mappings(
        Some(UserRole::InternalUserViewer),
        &[(UserRole::ProxyAdmin, &["admins"])],
    );
    let converted = convert(
        r#"{"role":"internal_user"}"#,
        &config(&jwt_handler, None, Some(&role_mappings), None),
        &[],
    )
    .unwrap();
    assert_eq!(converted.user_role, Some(UserRole::InternalUserViewer));
}

#[test]
fn the_role_claim_fills_a_missing_mapping() {
    let converted = plain(r#"{"role":["internal_user","proxy_admin"]}"#);
    assert_eq!(converted.user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn an_unmatched_group_falls_through_to_the_role_claim() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let role_mappings = mappings(None, &[(UserRole::ProxyAdmin, &["admins"])]);
    let converted = convert(
        r#"{"groups":["staff"],"role":"internal_user"}"#,
        &config(&jwt_handler, None, Some(&role_mappings), None),
        &[],
    )
    .unwrap();
    assert_eq!(converted.user_role, Some(UserRole::InternalUser));
}

#[test]
fn sso_teams_come_before_the_ordinary_handler_teams() {
    let sso = team_handler("team_ids");
    let ordinary = team_handler("jwt_teams");
    let converted = convert(
        r#"{"team_ids":["t1","t2"],"jwt_teams":["ordinary"]}"#,
        &config(&ordinary, Some(&sso), None, None),
        &[],
    )
    .unwrap();
    assert_eq!(converted.team_ids, strings(&["t1", "t2", "ordinary"]));
}

#[test]
fn teams_need_a_configured_field() {
    let converted = plain(r#"{"team_ids":["t1"]}"#);
    assert!(converted.team_ids.is_empty());
}

#[test]
fn a_db_mapping_appends_without_deduplicating() {
    let sso = team_handler("team_ids");
    let ordinary = team_handler("jwt_teams");
    let db_mapping = TeamMappings {
        team_ids_jwt_field: Some("db_teams".to_owned()),
    };
    let converted = convert(
        r#"{"team_ids":["t1","t2"],"jwt_teams":["ordinary"],"db_teams":["t1","t2","t3"]}"#,
        &config(&ordinary, Some(&sso), None, Some(&db_mapping)),
        &[],
    )
    .unwrap();
    assert_eq!(converted.team_ids, strings(&["t1", "t2", "t1", "t2", "t3"]));
}

#[test]
fn a_db_mapping_suppresses_the_ordinary_lookup() {
    let sso = team_handler("team_ids");
    let ordinary = team_handler("jwt_teams");
    let db_mapping = TeamMappings {
        team_ids_jwt_field: Some("db_teams".to_owned()),
    };
    let converted = convert(
        r#"{"team_ids":["t1"],"jwt_teams":["ordinary"]}"#,
        &config(&ordinary, Some(&sso), None, Some(&db_mapping)),
        &[],
    )
    .unwrap();
    assert_eq!(converted.team_ids, strings(&["t1"]));
}

#[test]
fn a_db_mapping_without_a_field_uses_the_ordinary_lookup() {
    let ordinary = team_handler("jwt_teams");
    let db_mapping = TeamMappings {
        team_ids_jwt_field: None,
    };
    let converted = convert(
        r#"{"jwt_teams":["ordinary"]}"#,
        &config(&ordinary, None, None, Some(&db_mapping)),
        &[],
    )
    .unwrap();
    assert_eq!(converted.team_ids, strings(&["ordinary"]));
}

#[test]
fn a_non_list_db_mapping_adds_nothing() {
    let sso = team_handler("team_ids");
    let ordinary = team_handler("jwt_teams");
    let db_mapping = TeamMappings {
        team_ids_jwt_field: Some("db_teams".to_owned()),
    };
    let converted = convert(
        r#"{"team_ids":["t1"],"jwt_teams":["ordinary"],"db_teams":"t1"}"#,
        &config(&ordinary, Some(&sso), None, Some(&db_mapping)),
        &[],
    )
    .unwrap();
    assert_eq!(converted.team_ids, strings(&["t1"]));
}

#[test]
fn a_db_mapping_list_must_hold_strings() {
    let jwt_handler = handler(JwtAuthSettings::default());
    let db_mapping = TeamMappings {
        team_ids_jwt_field: Some("db_teams".to_owned()),
    };
    let error = convert(
        r#"{"db_teams":["t1",7]}"#,
        &config(&jwt_handler, None, None, Some(&db_mapping)),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(&error, SsoClaimsError::Claim(ClaimError::Type { claim, .. }) if claim == "db_teams"),
        "{error:?}"
    );
}

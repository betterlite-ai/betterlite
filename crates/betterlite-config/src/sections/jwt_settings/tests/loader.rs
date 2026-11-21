use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rstest::rstest;

use super::super::Settings;
use super::super::construct::{Context, construct};
use super::super::error::JwtSettingsError;
use super::super::records::{Selector, UnregisteredClientBehavior};
use super::super::validator::{
    LoadFailure, ReturnAnnotation, UnregisteredLoader, ValidatorLoader, ValidatorRequest,
};
use super::parsed;
use crate::loader::{ConfigError, ConfigLoader};

fn load(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }.load(yaml)?.sections.jwt_settings)
}

fn nested(settings: &str) -> String {
    format!("general_settings:\n  litellm_jwtauth: {settings}\n")
}

const ALL_FIELDS: &str = "
general_settings:
  litellm_jwtauth:
    admin_jwt_scope: admin.scope
    admin_allowed_routes: [management_routes]
    team_id_jwt_field: team.id
    team_id_upsert: true
    team_ids_jwt_field: team.ids
    upsert_sso_user_to_team: true
    team_allowed_routes: [openai_routes]
    team_id_default: default-team
    team_alias_jwt_field: team.name
    org_id_jwt_field: org.id
    org_alias_jwt_field: org.name
    user_id_jwt_field: sub
    user_email_jwt_field: email
    user_allowed_email_domain: example.com
    user_roles_jwt_field: roles
    user_allowed_roles: [admin]
    user_id_upsert: true
    end_user_id_jwt_field: customer
    agent_id_jwt_field: azp
    mcp_client_id_jwt_field: client_id
    public_key_ttl: 60
    public_key_stale_ttl: 0
    public_allowed_routes: [public_routes, info_routes]
    enforce_rbac: true
    roles_jwt_field: realm.roles
    role_mappings: [{role: admin, internal_role: proxy_admin}]
    object_id_jwt_field: oid
    scope_mappings: [{scope: read, models: [gpt-4o]}]
    enforce_scope_based_access: true
    enforce_team_based_model_access: true
    jwt_litellm_role_map: [{jwt_role: admins, litellm_role: proxy_admin_viewer}]
    sync_user_role_and_teams: true
    oidc_userinfo_endpoint: https://idp/userinfo
    oidc_userinfo_enabled: true
    oidc_userinfo_cache_ttl: 30
    virtual_key_claim_field: client_id
    virtual_key_mapping_cache_ttl: 45
    unregistered_jwt_client_behavior: auto_register
    routing_overrides: [{iss: https://idp, scope: [mcp]}]
    team_claim_fallback: true
    fallback_to_db_teams: true
    issuers: [{issuer: https://idp, audience: api}]
";

#[test]
fn loads_route_and_team_fields() {
    let loaded = load(ALL_FIELDS, &[]).unwrap();
    assert_eq!(loaded.routes.admin_jwt_scope, "admin.scope");
    assert_eq!(loaded.routes.admin_allowed_routes, ["management_routes"]);
    assert_eq!(loaded.teams.team_id_jwt_field.as_deref(), Some("team.id"));
    assert!(loaded.teams.team_id_upsert && loaded.teams.upsert_sso_user_to_team);
    assert_eq!(loaded.teams.team_ids_jwt_field.as_deref(), Some("team.ids"));
    assert_eq!(loaded.routes.team_allowed_routes, ["openai_routes"]);
    assert_eq!(
        loaded.teams.team_id_default.as_deref(),
        Some("default-team")
    );
    assert_eq!(
        loaded.teams.team_alias_jwt_field.as_deref(),
        Some("team.name")
    );
    assert_eq!(
        loaded.routes.public_allowed_routes,
        ["public_routes", "info_routes"]
    );
}

#[test]
fn loads_claim_fields() {
    let loaded = load(ALL_FIELDS, &[]).unwrap();
    assert_eq!(loaded.claims.org_id_jwt_field.as_deref(), Some("org.id"));
    assert_eq!(
        loaded.claims.org_alias_jwt_field.as_deref(),
        Some("org.name")
    );
    assert_eq!(loaded.claims.user_id_jwt_field.as_deref(), Some("sub"));
    assert_eq!(loaded.claims.user_email_jwt_field.as_deref(), Some("email"));
    assert_eq!(
        loaded.claims.user_allowed_email_domain.as_deref(),
        Some("example.com")
    );
    assert_eq!(loaded.claims.user_roles_jwt_field.as_deref(), Some("roles"));
    assert_eq!(
        loaded.claims.user_allowed_roles,
        Some(vec!["admin".to_owned()])
    );
    assert!(loaded.claims.user_id_upsert);
    assert_eq!(
        loaded.claims.end_user_id_jwt_field.as_deref(),
        Some("customer")
    );
    assert_eq!(loaded.claims.agent_id_jwt_field.as_deref(), Some("azp"));
    assert_eq!(
        loaded.claims.mcp_client_id_jwt_field.as_deref(),
        Some("client_id")
    );
}

#[test]
fn loads_key_rbac_and_scope_fields() {
    let loaded = load(ALL_FIELDS, &[]).unwrap();
    assert_eq!(loaded.keys.public_key_ttl.to_bits(), 60.0_f64.to_bits());
    assert_eq!(
        loaded.keys.public_key_stale_ttl.to_bits(),
        0.0_f64.to_bits()
    );
    assert!(loaded.rbac.enforce_rbac);
    assert_eq!(loaded.rbac.roles_jwt_field.as_deref(), Some("realm.roles"));
    assert_eq!(loaded.rbac.role_mappings.as_ref().map(Vec::len), Some(1));
    assert_eq!(loaded.rbac.object_id_jwt_field.as_deref(), Some("oid"));
    assert_eq!(loaded.scopes.scope_mappings.as_ref().map(Vec::len), Some(1));
    assert!(loaded.scopes.enforce_scope_based_access && loaded.scopes.team_model_access);
    assert_eq!(loaded.custom_validate, None);
    assert_eq!(
        loaded.sync.jwt_litellm_role_map.as_ref().map(Vec::len),
        Some(1)
    );
}

#[test]
fn loads_userinfo_key_and_routing_fields() {
    let loaded = load(ALL_FIELDS, &[]).unwrap();
    assert!(loaded.sync.sync_user_role_and_teams && loaded.userinfo.oidc_userinfo_enabled);
    assert_eq!(
        loaded.userinfo.oidc_userinfo_endpoint.as_deref(),
        Some("https://idp/userinfo")
    );
    assert_eq!(
        loaded.userinfo.oidc_userinfo_cache_ttl.to_bits(),
        30.0_f64.to_bits()
    );
    assert_eq!(
        loaded.virtual_keys.virtual_key_claim_field.as_deref(),
        Some("client_id")
    );
    assert_eq!(
        loaded.virtual_keys.virtual_key_mapping_cache_ttl.to_bits(),
        45.0_f64.to_bits()
    );
    assert_eq!(
        loaded.virtual_keys.unregistered_client,
        UnregisteredClientBehavior::AutoRegister
    );
    let overrides = loaded.routing.routing_overrides.as_ref().unwrap();
    assert_eq!(
        overrides[0].scope,
        Some(Selector::Many(vec!["mcp".to_owned()]))
    );
    assert!(loaded.routing.team_claim_fallback && loaded.routing.fallback_to_db_teams);
    assert_eq!(
        loaded.issuer_config(Some("https://idp")).unwrap().audience,
        Some(Selector::One("api".to_owned()))
    );
}

#[rstest]
#[case::absent("")]
#[case::null_section("general_settings:\n  litellm_jwtauth: null\n")]
#[case::empty_section("general_settings:\n  litellm_jwtauth: {}\n")]
fn absent_section_loads_defaults(#[case] yaml: &str) {
    assert_eq!(load(yaml, &[]).unwrap(), Settings::default());
}

#[test]
fn env_top_level_resolution() {
    let yaml = nested(
        "{team_id_jwt_field: os.environ/TEAM, enforce_rbac: os.environ/RBAC, public_key_ttl: os.environ/TTL, unregistered_jwt_client_behavior: os.environ/BEHAVIOR, jwt_client_id_field: os.environ/CLIENT, user_email_jwt_field: os.environ/EMPTY, team_allowed_routes: [os.environ/TEAM], issuers: [{issuer: os.environ/TEAM, audience: x}]}",
    );
    let environment = [
        ("TEAM", "groups.0"),
        ("RBAC", "yes"),
        ("TTL", "90"),
        ("BEHAVIOR", "reject"),
        ("CLIENT", "azp"),
        ("EMPTY", ""),
    ];
    let loaded = load(&yaml, &environment).unwrap();
    assert_eq!(loaded.teams.team_id_jwt_field.as_deref(), Some("groups.0"));
    assert!(loaded.rbac.enforce_rbac);
    assert_eq!(loaded.keys.public_key_ttl.to_bits(), 90.0_f64.to_bits());
    assert_eq!(
        loaded.virtual_keys.unregistered_client,
        UnregisteredClientBehavior::Reject
    );
    assert_eq!(
        loaded.virtual_keys.virtual_key_claim_field.as_deref(),
        Some("azp")
    );
    assert_eq!(loaded.claims.user_email_jwt_field, None);
    assert_eq!(loaded.routes.team_allowed_routes, ["os.environ/TEAM"]);
    assert_eq!(loaded.routing.issuers.unwrap()[0].issuer, "os.environ/TEAM");
    let missing = load(&nested("{org_id_jwt_field: os.environ/ORG}"), &[]).unwrap_err();
    assert!(
        matches!(&missing, ConfigError::MissingEnvironment { variable, path } if variable == "ORG" && path == "general_settings.litellm_jwtauth.org_id_jwt_field"),
        "{missing}"
    );
    let unparsed = load(
        &nested("{public_key_ttl: os.environ/TTL}"),
        &[("TTL", "soon")],
    )
    .unwrap_err();
    assert!(
        matches!(unparsed, ConfigError::Environment { .. }),
        "{unparsed}"
    );
}

#[rstest]
#[case::text_for_flag("{user_id_upsert: 'yes'}", "user_id_upsert")]
#[case::number_for_text("{team_id_jwt_field: 5}", "team_id_jwt_field")]
#[case::list_for_text("{user_email_jwt_field: [email]}", "user_email_jwt_field")]
#[case::text_for_number("{oidc_userinfo_cache_ttl: soon}", "oidc_userinfo_cache_ttl")]
#[case::text_for_list("{admin_allowed_routes: info_routes}", "admin_allowed_routes")]
#[case::record_field("{issuers: [{issuer: a, audience: 5}]}", "issuers.0.audience")]
#[case::issuer_audience("{issuers: [{issuer: a}]}", "issuers.0")]
#[case::paired_roles("{user_roles_jwt_field: roles}", "user_allowed_roles")]
#[case::object_id("{object_id_jwt_field: oid}", "role_mappings")]
#[case::scopes("{scope_mappings: []}", "scope_mappings")]
#[case::unregistered_validator("{custom_validate: hooks.validate}", "custom_validate")]
fn invalid_types_reject(#[case] settings: &str, #[case] field: &str) {
    let error = load(&nested(settings), &[]).unwrap_err();
    let expected = format!("general_settings.litellm_jwtauth.{field}");
    assert!(
        matches!(&error, ConfigError::InvalidScalar { path, value: false } if *path == expected),
        "{error}"
    );
}

#[test]
fn unknown_keys_fail_the_load() {
    let error = load(
        &nested("{team_jwt_scope: team, user_id_jwt_field: sub}"),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if paths == &["general_settings.litellm_jwtauth.team_jwt_scope"]),
        "{error}"
    );
}

#[derive(Default)]
struct RecordingLoader {
    answer: Option<ReturnAnnotation>,
    requests: Mutex<Vec<(String, Option<PathBuf>)>>,
}

impl ValidatorLoader for RecordingLoader {
    fn load(&self, request: &ValidatorRequest<'_>) -> Result<ReturnAnnotation, LoadFailure> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push((
                request.module.to_owned(),
                request.config_file_path.map(Path::to_path_buf),
            ));
        }
        self.answer.clone().ok_or_else(|| LoadFailure::Import {
            instance: "validate".to_owned(),
            module: "hooks".to_owned(),
        })
    }
}

fn construct_with(
    yaml: &str,
    loader: &dyn ValidatorLoader,
    config_file_path: Option<&Path>,
) -> Result<Settings, JwtSettingsError> {
    let lookup = BTreeMap::from([("VALIDATOR".to_owned(), "hooks.from_env".to_owned())]);
    construct(
        parsed(yaml),
        &Context {
            loader,
            config_file_path,
            lookup: &lookup,
        },
    )
}

#[rstest]
#[case::local_module(
    "{custom_validate: hooks.validate}",
    Some("/etc/proxy/config.yaml"),
    "hooks.validate"
)]
#[case::without_config_file("{custom_validate: hooks.validate}", None, "hooks.validate")]
#[case::remote_with_config_file(
    "{custom_validate: 's3://bucket/hooks.validate'}",
    Some("/etc/proxy/config.yaml"),
    "s3://bucket/hooks.validate"
)]
#[case::reference("{custom_validate: os.environ/VALIDATOR}", None, "hooks.from_env")]
fn custom_loader_receives_context(
    #[case] yaml: &str,
    #[case] config_file: Option<&str>,
    #[case] module: &str,
) {
    let loader = RecordingLoader {
        answer: Some(ReturnAnnotation::LiteralTrue),
        requests: Mutex::default(),
    };
    let built = construct_with(yaml, &loader, config_file.map(Path::new)).unwrap();
    assert_eq!(built.custom_validate.as_deref(), Some(module));
    let requests = loader.requests.into_inner().unwrap();
    assert_eq!(
        requests,
        [(module.to_owned(), config_file.map(PathBuf::from))]
    );
}

#[rstest]
#[case::s3("{custom_validate: 's3://bucket/hooks.validate', extra: 1}")]
#[case::gcs("{custom_validate: 'gcs://bucket/hooks.validate', extra: 1}")]
fn remote_module_needs_the_config_file(#[case] yaml: &str) {
    let loader = RecordingLoader {
        answer: Some(ReturnAnnotation::LiteralTrue),
        requests: Mutex::default(),
    };
    let error = construct_with(yaml, &loader, None).unwrap_err();
    assert!(
        matches!(
            error,
            JwtSettingsError::ValidatorLoad {
                source: LoadFailure::RemoteWithoutConfigFile
            }
        ),
        "{error}"
    );
    assert!(error.to_string().starts_with(
        "Remote module loading (s3://, gcs://) is only permitted from the config-file load path."
    ));
    assert!(loader.requests.into_inner().unwrap().is_empty());
}

#[test]
fn custom_loader_failure_precedes_unknown_key() {
    let yaml = "{custom_validate: hooks.validate, extra: 1, user_roles_jwt_field: roles, team_id_jwt_field: 5}";
    let error = construct_with(yaml, &RecordingLoader::default(), None).unwrap_err();
    assert_eq!(error.to_string(), "Could not import validate from hooks");
    let unregistered = construct_with(yaml, &UnregisteredLoader, None).unwrap_err();
    assert_eq!(
        unregistered.to_string(),
        "custom_validate hooks.validate needs a validator loader and none is registered"
    );
    let accepted = RecordingLoader {
        answer: Some(ReturnAnnotation::LiteralTrue),
        requests: Mutex::default(),
    };
    assert!(
        construct_with(yaml, &accepted, None)
            .unwrap_err()
            .to_string()
            .starts_with("Invalid arguments provided: extra.")
    );
}

#[rstest]
#[case::plain_bool("<class 'bool'>")]
#[case::unannotated("None")]
#[case::literal_false("typing.Literal[False]")]
#[case::union("bool | None")]
fn custom_annotation_must_be_literal_true(#[case] rendered: &str) {
    let loader = RecordingLoader {
        answer: Some(ReturnAnnotation::Other(rendered.to_owned())),
        requests: Mutex::default(),
    };
    let error =
        construct_with("{custom_validate: hooks.validate, extra: 1}", &loader, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Custom validator must be annotated to return Literal[True], got {rendered}")
    );
}

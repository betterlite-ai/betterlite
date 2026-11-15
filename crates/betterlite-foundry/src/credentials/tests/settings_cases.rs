use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret, SecretString};

use crate::credentials::entra::{EntraTokenRequest, get_azure_ai_entra_token};
use crate::credentials::error::CredentialsError;
use crate::credentials::params::EntraParams;

use super::builders::{accessors, environment, object};
use super::support::FixtureSources;

pub(super) fn foundry_settings() {
    accessor_precedence();
    entra_tokens();
    deployment_environment_precedence();
    empty_scope_is_kept();
    invalid_parameters();
}

fn revealed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

fn accessor_precedence() {
    let global_key = SecretString::from("global-key".to_owned());
    let env_key = SecretString::from("env-key".to_owned());
    let explicit_key = SecretString::from("explicit-key".to_owned());
    let all = accessors(
        Some("https://global.example"),
        Some(&global_key),
        Some("https://env.example"),
        Some(&env_key),
    );
    assert_eq!(
        all.api_base(Some("https://explicit.example")),
        Some("https://explicit.example".to_owned())
    );
    assert_eq!(
        all.api_base(None),
        Some("https://global.example".to_owned())
    );
    assert_eq!(
        revealed(all.api_key(Some(&explicit_key)).as_ref()),
        Some("explicit-key")
    );
    assert_eq!(revealed(all.api_key(None).as_ref()), Some("global-key"));
    let from_env = accessors(None, None, Some("https://env.example"), Some(&env_key));
    assert_eq!(
        from_env.api_base(None),
        Some("https://env.example".to_owned())
    );
    assert_eq!(revealed(from_env.api_key(None).as_ref()), Some("env-key"));
    let bare = accessors(None, None, None, None);
    assert!(bare.api_base(None).is_none());
    assert!(bare.api_key(None).is_none());
    let empty = SecretString::from(String::new());
    assert_eq!(from_env.api_base(Some("")), Some(String::new()));
    assert_eq!(revealed(from_env.api_key(Some(&empty)).as_ref()), Some(""));
}

fn entra_tokens() {
    absent_tokens();
    configured_tokens();
}

fn absent_tokens() {
    let empty_env = environment(&[]);
    let sources = FixtureSources::new();
    assert!(
        get_azure_ai_entra_token(&EntraTokenRequest {
            params: None,
            environment: &empty_env,
            refresh_enabled: false,
            sources: &sources,
        })
        .unwrap()
        .is_none()
    );
    let empty = object([]);
    assert!(
        get_azure_ai_entra_token(&EntraTokenRequest {
            params: Some(&empty),
            environment: &empty_env,
            refresh_enabled: false,
            sources: &sources,
        })
        .unwrap()
        .is_none()
    );
    assert!(sources.calls().is_empty());
}

fn configured_tokens() {
    let empty_env = environment(&[]);
    let from_env = environment(&[("AZURE_AD_TOKEN", "env-token")]);
    let env_sources = FixtureSources::new();
    let token = get_azure_ai_entra_token(&EntraTokenRequest {
        params: None,
        environment: &from_env,
        refresh_enabled: false,
        sources: &env_sources,
    })
    .unwrap()
    .unwrap();
    assert_eq!(token.expose_secret(), "env-token");
    assert!(env_sources.calls().is_empty());
    let params = grant_params();
    let grant_sources = FixtureSources::new();
    let grant = get_azure_ai_entra_token(&EntraTokenRequest {
        params: Some(&params),
        environment: &empty_env,
        refresh_enabled: false,
        sources: &grant_sources,
    })
    .unwrap()
    .unwrap();
    assert_eq!(grant.expose_secret(), "client-secret-token");
    assert_eq!(
        grant_sources.calls(),
        vec![format!(
            "client_secret tenant=tenant client=client secret=secret scope={DEFAULT_SCOPE}"
        )]
    );
}

fn deployment_environment_precedence() {
    let env = environment(&[("AZURE_SCOPE", ""), ("AZURE_TENANT_ID", "env-tenant")]);
    let explicit = object([
        ("tenant_id", Json::Text(String::new())),
        ("azure_scope", Json::Text(String::new())),
    ]);
    let settings = EntraParams::from_mapping(Some(&explicit))
        .unwrap()
        .deployment_settings(&env);
    assert_eq!(settings.tenant_id.as_deref(), Some("env-tenant"));
    assert_eq!(settings.azure_scope, "");
    let absent = object([("tenant_id", Json::Null), ("client_id", Json::Null)]);
    let from_absent = EntraParams::from_mapping(Some(&absent))
        .unwrap()
        .deployment_settings(&env);
    assert_eq!(from_absent.tenant_id.as_deref(), Some("env-tenant"));
    assert!(from_absent.client_id.is_none());
    assert_eq!(from_absent.azure_scope, "");
    let bare = EntraParams::default().deployment_settings(&environment(&[]));
    assert_eq!(bare.azure_scope, DEFAULT_SCOPE);
}

fn empty_scope_is_kept() {
    let from_env = environment(&[("AZURE_SCOPE", "")]);
    let params = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("secret".to_owned())),
        ("azure_scope", Json::Text(String::new())),
    ]);
    let sources = FixtureSources::new();
    get_azure_ai_entra_token(&EntraTokenRequest {
        params: Some(&params),
        environment: &from_env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap();
    assert_eq!(
        sources.calls(),
        vec!["client_secret tenant=tenant client=client secret=secret scope=".to_owned()]
    );
}

fn grant_params() -> Object {
    object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("secret".to_owned())),
    ])
}

fn invalid_parameters() {
    let environment = environment(&[]);
    let sources = FixtureSources::new();
    let bad_tenant = object([("tenant_id", Json::Bool(true))]);
    let error = get_azure_ai_entra_token(&EntraTokenRequest {
        params: Some(&bad_tenant),
        environment: &environment,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err();
    assert!(matches!(
        error,
        CredentialsError::InvalidParam { field: "tenant_id" }
    ));
    assert_eq!(
        error.to_string(),
        "litellm_params.tenant_id must be a string"
    );
    let bad_secret = object([("client_secret", Json::List(Vec::new()))]);
    let secret_error = get_azure_ai_entra_token(&EntraTokenRequest {
        params: Some(&bad_secret),
        environment: &environment,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err();
    assert!(matches!(
        secret_error,
        CredentialsError::InvalidParam {
            field: "client_secret"
        }
    ));
    assert!(sources.calls().is_empty());
}

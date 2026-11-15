use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::ExposeSecret as _;

use crate::credentials::agent::{
    get_azure_ai_agent_entra_token, has_azure_entra_params, resolve_agent_auth_header,
};
use crate::credentials::error::CredentialsError;
use crate::credentials::{AZURE_AI_AGENTS_SCOPE, AZURE_ENTRA_CREDENTIAL_HELP};

use super::builders::{agent_request, environment, headers, object};
use super::support::{FixtureSources, secrets};

pub(super) fn agent_entra_scope() {
    client_secret_grant();
    explicit_agent_scope();
    configured_grants();
    static_token();
    unresolved();
}

fn client_secret_grant() {
    let environment = environment(&[]);
    let secrets = secrets();
    let sources = FixtureSources::new();
    let params = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("secret".to_owned())),
    ]);
    let token =
        get_azure_ai_agent_entra_token(&agent_request(&params, &environment, &secrets, &sources))
            .unwrap();
    assert_eq!(token.expose_secret(), "client-secret-token");
    assert_eq!(
        sources.calls(),
        vec![format!(
            "client_secret tenant=tenant client=client secret=secret scope={AZURE_AI_AGENTS_SCOPE}"
        )]
    );
    let header =
        resolve_agent_auth_header(&agent_request(&params, &environment, &secrets, &sources))
            .unwrap();
    assert_eq!(
        header,
        headers(&[("Authorization", "Bearer client-secret-token")])
    );
}

fn explicit_agent_scope() {
    let environment = environment(&[]);
    let secrets = secrets();
    let scoped = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("secret".to_owned())),
        (
            "azure_scope",
            Json::Text("https://custom/.default".to_owned()),
        ),
    ]);
    let sources = FixtureSources::new();
    get_azure_ai_agent_entra_token(&agent_request(&scoped, &environment, &secrets, &sources))
        .unwrap();
    assert!(
        sources.calls()[0].contains("scope=https://custom/.default"),
        "{:?}",
        sources.calls()
    );
}

fn assert_grant(params: &Object, expected_token: &str, expected_call: &str) {
    let environment = environment(&[]);
    let secrets = secrets();
    let sources = FixtureSources::new();
    let token =
        get_azure_ai_agent_entra_token(&agent_request(params, &environment, &secrets, &sources))
            .unwrap();
    assert_eq!(token.expose_secret(), expected_token);
    assert_eq!(sources.calls(), vec![expected_call.to_owned()]);
}

fn configured_grants() {
    let cases = [
        (
            object([
                ("client_id", Json::Text("client".to_owned())),
                ("azure_username", Json::Text("user".to_owned())),
                ("azure_password", Json::Text("password".to_owned())),
            ]),
            "username-token",
            format!(
                "username_password client=client user=user password=password scope={AZURE_AI_AGENTS_SCOPE} tenant=None"
            ),
        ),
        (
            object([
                ("azure_ad_token", Json::Text("oidc/assertion".to_owned())),
                ("tenant_id", Json::Text("tenant".to_owned())),
                ("client_id", Json::Text("client".to_owned())),
            ]),
            "oidc-token",
            format!(
                "oidc assertion=oidc/assertion client={:?} tenant={:?} scope={:?}",
                Some("client"),
                Some("tenant"),
                Some(AZURE_AI_AGENTS_SCOPE)
            ),
        ),
    ];
    for (params, token, call) in &cases {
        assert_grant(params, token, call);
    }
}

fn static_token() {
    let environment = environment(&[]);
    let secrets = secrets();
    let sources = FixtureSources::new();
    let params = object([("azure_ad_token", Json::Text("static-token".to_owned()))]);
    let token =
        get_azure_ai_agent_entra_token(&agent_request(&params, &environment, &secrets, &sources))
            .unwrap();
    assert_eq!(token.expose_secret(), "static-token");
    assert!(sources.calls().is_empty());
    let with_identity = object([
        ("azure_ad_token", Json::Text("static-token".to_owned())),
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
    ]);
    let pinned = get_azure_ai_agent_entra_token(&agent_request(
        &with_identity,
        &environment,
        &secrets,
        &sources,
    ))
    .unwrap();
    assert_eq!(pinned.expose_secret(), "static-token");
    assert!(sources.calls().is_empty());
}

fn unresolved() {
    let environment = environment(&[]);
    let secrets = secrets();
    let sources = FixtureSources::new();
    let empty = object([]);
    let error =
        get_azure_ai_agent_entra_token(&agent_request(&empty, &environment, &secrets, &sources))
            .unwrap_err();
    assert!(matches!(
        error,
        CredentialsError::AgentTokenUnresolved { .. }
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "Azure AI agent Entra ID credentials did not resolve to a token. {AZURE_ENTRA_CREDENTIAL_HELP}"
        )
    );
    let partial_oidc = object([
        ("azure_ad_token", Json::Text("oidc/assertion".to_owned())),
        ("tenant_id", Json::Text("tenant".to_owned())),
    ]);
    let partial = get_azure_ai_agent_entra_token(&agent_request(
        &partial_oidc,
        &environment,
        &secrets,
        &sources,
    ))
    .unwrap_err();
    assert!(matches!(
        partial,
        CredentialsError::AgentTokenUnresolved { .. }
    ));
    assert!(sources.calls().is_empty());
}

pub(super) fn agent_config_only_credentials() {
    env_references();
    non_string_values_ignored();
    entra_param_truthiness();
}

fn env_references() {
    let references = environment(&[
        ("TENANT", "tenant-env"),
        ("CLIENT", "client-env"),
        ("SECRET", "secret-env"),
        ("SCOPE", "https://env/.default"),
    ]);
    let secrets = secrets();
    let sources = FixtureSources::new();
    let params = object([
        ("tenant_id", Json::Text("os.environ/TENANT".to_owned())),
        ("client_id", Json::Text("os.environ/CLIENT".to_owned())),
        ("client_secret", Json::Text("os.environ/SECRET".to_owned())),
        ("azure_scope", Json::Text("os.environ/SCOPE".to_owned())),
    ]);
    let token =
        get_azure_ai_agent_entra_token(&agent_request(&params, &references, &secrets, &sources))
            .unwrap();
    assert_eq!(token.expose_secret(), "client-secret-token");
    assert_eq!(
        sources.calls(),
        vec![
            "client_secret tenant=tenant-env client=client-env secret=secret-env scope=https://env/.default"
                .to_owned()
        ]
    );
    let missing = object([(
        "azure_ad_token",
        Json::Text("os.environ/MISSING".to_owned()),
    )]);
    let missing_error =
        get_azure_ai_agent_entra_token(&agent_request(&missing, &references, &secrets, &sources))
            .unwrap_err();
    assert!(matches!(
        missing_error,
        CredentialsError::AgentTokenUnresolved { .. }
    ));
}

fn non_string_values_ignored() {
    let environment = environment(&[("SECRET", "")]);
    let secrets = secrets();
    let sources = FixtureSources::new();
    let non_string = object([
        ("tenant_id", Json::Bool(true)),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("secret".to_owned())),
    ]);
    let ignored = get_azure_ai_agent_entra_token(&agent_request(
        &non_string,
        &environment,
        &secrets,
        &sources,
    ))
    .unwrap_err();
    assert!(matches!(
        ignored,
        CredentialsError::AgentTokenUnresolved { .. }
    ));
    let empty_secret = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
        ("client_secret", Json::Text("os.environ/SECRET".to_owned())),
        ("azure_ad_token", Json::List(Vec::new())),
    ]);
    let empty = get_azure_ai_agent_entra_token(&agent_request(
        &empty_secret,
        &environment,
        &secrets,
        &sources,
    ))
    .unwrap_err();
    assert!(matches!(
        empty,
        CredentialsError::AgentTokenUnresolved { .. }
    ));
    assert!(sources.calls().is_empty());
}

fn entra_param_truthiness() {
    assert!(!has_azure_entra_params(None));
    let empty = object([]);
    assert!(!has_azure_entra_params(Some(&empty)));
    for key in ["azure_ad_token", "client_secret", "azure_password"] {
        let present = object([(key, Json::Text("value".to_owned()))]);
        assert!(has_azure_entra_params(Some(&present)), "{key}");
    }
    let identity_only = object([
        ("tenant_id", Json::Text("tenant".to_owned())),
        ("client_id", Json::Text("client".to_owned())),
    ]);
    assert!(!has_azure_entra_params(Some(&identity_only)));
    let falsy = [
        ("azure_ad_token", Json::Text(String::new())),
        ("client_secret", Json::Bool(false)),
        ("azure_password", Json::List(Vec::new())),
        ("azure_scope", Json::Text("scope".to_owned())),
    ];
    for (key, value) in falsy {
        let params = object([(key, value)]);
        assert!(!has_azure_entra_params(Some(&params)), "{key}");
    }
    let truthy = object([("client_secret", Json::Bool(true))]);
    assert!(has_azure_entra_params(Some(&truthy)));
}

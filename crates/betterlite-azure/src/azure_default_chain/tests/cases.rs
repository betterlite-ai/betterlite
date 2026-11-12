use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use secrecy::ExposeSecret as _;

use super::super::environment::DEFAULT_SCOPE;
use super::super::error::{ENVIRONMENT_UNAVAILABLE, TROUBLESHOOT_URL};
use super::super::plan::ENVIRONMENT_PASSWORD_WARNING;
use super::super::provider::{AzureTokenProvider, try_default_credential};
use super::super::selection::selection;
use super::{FixtureGrants, Script, capture, environment, provider};

const CLIENT_SECRET: &str = "ClientSecretCredential";
const CERTIFICATE: &str = "CertificateCredential";
const USERNAME_PASSWORD: &str = "UsernamePasswordCredential";
const MANAGED: &str = "ManagedIdentityCredential";
const WORKLOAD: &str = "WorkloadIdentityCredential";
const DEPLOYMENT: AzureCredentialType = AzureCredentialType::DeploymentIdentityCredential;
const DEFAULT: AzureCredentialType = AzureCredentialType::DefaultAzureCredential;

fn inferred(variables: &[(&str, &str)]) -> AzureCredentialType {
    environment(variables).infer_credential_type()
}

fn default_provider<'grants>(
    variables: &[(&str, &str)],
    grants: &'grants FixtureGrants,
) -> AzureTokenProvider<'grants, FixtureGrants> {
    let env = environment(variables);
    provider(None, Some(DEFAULT), &env, grants).unwrap()
}

pub(super) fn explicit_credential() {
    explicit_wins_over_environment();
    environment_credential_used();
    empty_environment_credential_falls_through();
    scope_resolution();
}

fn explicit_wins_over_environment() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", CLIENT_SECRET),
        ("AZURE_CLIENT_ID", "explicit-client"),
        ("AZURE_CLIENT_SECRET", "explicit-secret"),
        ("AZURE_TENANT_ID", "explicit-tenant"),
        ("AZURE_CERTIFICATE_PATH", "/certs/explicit.pem"),
    ]);
    let (result, logs) = capture(|| {
        provider(
            None,
            Some(AzureCredentialType::CertificateCredential),
            &env,
            &grants,
        )
    });
    let token_provider = result.unwrap();
    assert_eq!(
        token_provider.credential,
        AzureCredentialType::CertificateCredential
    );
    assert_eq!(token_provider.scope, DEFAULT_SCOPE);
    assert_eq!(grants.records().len(), 1);
    assert_eq!(grants.records()[0].credential, CERTIFICATE);
    assert_eq!(
        grants.records()[0].certificate_path.as_deref(),
        Some("/certs/explicit.pem")
    );
    assert!(logs.iter().any(|line| {
        line == "For Azure AD Token Provider, choosing credential type: CertificateCredential"
    }));
}

fn environment_credential_used() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", MANAGED),
        ("AZURE_CLIENT_ID", "env-client"),
    ]);
    let token_provider = provider(None, None, &env, &grants).unwrap();
    assert_eq!(
        token_provider.credential,
        AzureCredentialType::ManagedIdentityCredential
    );
    assert_eq!(grants.records()[0].client_id.as_deref(), Some("env-client"));
}

fn empty_environment_credential_falls_through() {
    let grants = FixtureGrants::new();
    let token_provider = provider(
        None,
        None,
        &environment(&[
            ("AZURE_CREDENTIAL", ""),
            ("AZURE_CLIENT_ID", "inferred-client"),
            ("AZURE_CLIENT_SECRET", "inferred-secret"),
            ("AZURE_TENANT_ID", "inferred-tenant"),
        ]),
        &grants,
    )
    .unwrap();
    let record = &grants.records()[0];
    assert_eq!(record.client_secret.as_deref(), Some("inferred-secret"));
    assert_eq!(
        token_provider.credential,
        AzureCredentialType::ClientSecretCredential
    );
}

fn scope_resolution() {
    let env = environment(&[
        ("AZURE_CREDENTIAL", MANAGED),
        ("AZURE_CLIENT_ID", "scope-client"),
        ("AZURE_SCOPE", "env-scope"),
    ]);
    let grants = FixtureGrants::new();
    assert_eq!(
        provider(None, None, &env, &grants).unwrap().scope,
        "env-scope"
    );
    assert_eq!(provider(Some(""), None, &env, &grants).unwrap().scope, "");
    assert_eq!(
        provider(Some("explicit-scope"), None, &env, &grants)
            .unwrap()
            .scope,
        "explicit-scope"
    );
    let empty = environment(&[
        ("AZURE_CREDENTIAL", MANAGED),
        ("AZURE_CLIENT_ID", "scope-client"),
        ("AZURE_SCOPE", ""),
    ]);
    assert_eq!(
        provider(None, None, &empty, &grants).unwrap().scope,
        DEFAULT_SCOPE
    );
}

pub(super) fn client_secret_inference() {
    inference_order();
    inference_truthiness();
    provider_follows_inference();
}

fn inference_order() {
    assert_eq!(
        inferred(&[
            ("AZURE_CLIENT_ID", "c"),
            ("AZURE_CLIENT_SECRET", "s"),
            ("AZURE_TENANT_ID", "t"),
        ]),
        AzureCredentialType::ClientSecretCredential
    );
    // an incomplete triplet leaves the client id to the managed identity branch
    assert_eq!(
        inferred(&[("AZURE_CLIENT_ID", "c"), ("AZURE_CLIENT_SECRET", "s")]),
        AzureCredentialType::ManagedIdentityCredential
    );
    // a token file selects the default credential even without tenant and client
    assert_eq!(
        inferred(&[("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token")]),
        AzureCredentialType::DefaultAzureCredential
    );
    assert_eq!(
        inferred(&[
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
            ("AZURE_CLIENT_ID", "c"),
        ]),
        AzureCredentialType::DefaultAzureCredential
    );
}

fn inference_truthiness() {
    assert_eq!(
        inferred(&[("AZURE_CERTIFICATE_PASSWORD", "p")]),
        AzureCredentialType::CertificateCredential
    );
    assert_eq!(
        inferred(&[("AZURE_CERTIFICATE_PATH", "/certs/azure.pem")]),
        AzureCredentialType::CertificateCredential
    );
    assert_eq!(inferred(&[]), AzureCredentialType::DefaultAzureCredential);
    assert_eq!(
        inferred(&[
            ("AZURE_CLIENT_ID", ""),
            ("AZURE_CLIENT_SECRET", ""),
            ("AZURE_TENANT_ID", ""),
            ("AZURE_FEDERATED_TOKEN_FILE", ""),
            ("AZURE_CERTIFICATE_PATH", ""),
        ]),
        AzureCredentialType::DefaultAzureCredential
    );
}

fn provider_follows_inference() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CLIENT_ID", "inferred-client"),
        ("AZURE_CLIENT_SECRET", "inferred-secret"),
        ("AZURE_TENANT_ID", "inferred-tenant"),
    ]);
    let token_provider = provider(None, None, &env, &grants).unwrap();
    assert_eq!(
        token_provider.credential,
        AzureCredentialType::ClientSecretCredential
    );
    let record = &grants.records()[0];
    assert_eq!(record.credential, CLIENT_SECRET);
    assert_eq!(record.client_id.as_deref(), Some("inferred-client"));
    assert_eq!(record.client_secret.as_deref(), Some("inferred-secret"));
    assert_eq!(record.tenant_id.as_deref(), Some("inferred-tenant"));
}

pub(super) fn client_id_before_certificate() {
    inference_prefers_the_client_id();
    missing_arguments_fail_before_construction();
    construction_failure_is_reported();
}

fn inference_prefers_the_client_id() {
    assert_eq!(
        inferred(&[
            ("AZURE_CLIENT_ID", "c"),
            ("AZURE_CERTIFICATE_PATH", "/certs/azure.pem"),
        ]),
        AzureCredentialType::ManagedIdentityCredential
    );
    assert_eq!(
        inferred(&[("AZURE_CERTIFICATE_PASSWORD", "p")]),
        AzureCredentialType::CertificateCredential
    );
    assert_eq!(
        inferred(&[("AZURE_CERTIFICATE_PATH", "/certs/azure.pem")]),
        AzureCredentialType::CertificateCredential
    );
}

fn missing_arguments_fail_before_construction() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", CERTIFICATE),
        ("AZURE_CERTIFICATE_PATH", "/certs/azure.pem"),
    ]);
    let error = provider(None, None, &env, &grants).unwrap_err();
    assert_eq!(error.category(), "missing_environment");
    assert_eq!(error.detail().0, Some("AZURE_CLIENT_ID"));
    assert!(grants.records().is_empty());
}

fn construction_failure_is_reported() {
    let grants = FixtureGrants::new().construct_error(CERTIFICATE, "fixture construction failure");
    let env = environment(&[
        ("AZURE_CREDENTIAL", CERTIFICATE),
        ("AZURE_CLIENT_ID", "cert-client"),
        ("AZURE_TENANT_ID", "cert-tenant"),
        ("AZURE_CERTIFICATE_PATH", "/certs/azure.pem"),
    ]);
    let error = provider(None, None, &env, &grants).unwrap_err();
    assert_eq!(error.category(), "construction");
    assert_eq!(error.to_string(), "fixture construction failure");
    assert!(grants.records().is_empty());
}

fn deployment_provider<'grants>(
    variables: &[(&str, &str)],
    grants: &'grants FixtureGrants,
) -> AzureTokenProvider<'grants, FixtureGrants> {
    let env = environment(variables);
    provider(None, Some(DEPLOYMENT), &env, grants).unwrap()
}

pub(super) fn deployment_chain_order() {
    eager_construction_and_lazy_tokens();
    auth_failure_stops_before_the_system_identity();
    no_client_id_builds_one_link();
    no_workload_variables_build_two_links();
}

fn eager_construction_and_lazy_tokens() {
    let grants = FixtureGrants::new()
        .script(
            WORKLOAD,
            Script::Unavailable("workload unavailable".to_owned()),
        )
        .script(
            "ManagedIdentityCredential:user",
            Script::Token("user-token".to_owned()),
        );
    let token_provider = deployment_provider(
        &[
            ("AZURE_CLIENT_ID", "pod-client"),
            ("AZURE_TENANT_ID", "pod-tenant"),
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ],
        &grants,
    );
    let records = grants.records();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].credential, WORKLOAD);
    assert_eq!(records[0].token_file.as_deref(), Some("/var/run/token"));
    assert_eq!(records[1].client_id.as_deref(), Some("pod-client"));
    assert_eq!(records[2].client_id, None);
    assert!(grants.calls().is_empty());
    assert_eq!(
        token_provider.token().unwrap().token.expose_secret(),
        "user-token"
    );
    assert_eq!(
        grants.calls(),
        vec![
            WORKLOAD.to_owned(),
            "ManagedIdentityCredential:user".to_owned()
        ]
    );
}

fn auth_failure_stops_before_the_system_identity() {
    let grants = FixtureGrants::new()
        .script(
            WORKLOAD,
            Script::Authentication("workload denied".to_owned()),
        )
        .script(
            "ManagedIdentityCredential:user",
            Script::Token("user-token".to_owned()),
        );
    let token_provider = deployment_provider(
        &[
            ("AZURE_CLIENT_ID", "pod-client"),
            ("AZURE_TENANT_ID", "pod-tenant"),
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ],
        &grants,
    );
    let error = token_provider.token().unwrap_err();
    assert_eq!(error.category(), "client_authentication");
    assert!(
        error
            .to_string()
            .starts_with("ChainedTokenCredential failed")
    );
    assert_eq!(grants.calls(), vec![WORKLOAD.to_owned()]);
}

fn no_client_id_builds_one_link() {
    let grants = FixtureGrants::new().script(
        "ManagedIdentityCredential:system",
        Script::Token("system-token".to_owned()),
    );
    let token_provider = deployment_provider(
        &[
            ("AZURE_TENANT_ID", "pod-tenant"),
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ],
        &grants,
    );
    let records = grants.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].client_id, None);
    assert_eq!(
        token_provider.token().unwrap().token.expose_secret(),
        "system-token"
    );
}

fn no_workload_variables_build_two_links() {
    let grants = FixtureGrants::new();
    let token_provider = deployment_provider(&[("AZURE_CLIENT_ID", "pod-client")], &grants);
    let records = grants.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].client_id.as_deref(), Some("pod-client"));
    assert_eq!(records[1].client_id, None);
    token_provider.token().unwrap();
}

pub(super) fn default_chain_fallback() {
    unavailable_links_fall_through();
    environment_token_runs_first();
    empty_values_still_select_environment();
    authentication_error_stops_the_chain();
}

fn unavailable_links_fall_through() {
    let grants = FixtureGrants::new().script(
        "ManagedIdentityCredential:system",
        Script::Token("system-token".to_owned()),
    );
    let token_provider = default_provider(&[], &grants);
    assert!(token_provider.warnings.is_empty());
    assert_eq!(
        token_provider.token().unwrap().token.expose_secret(),
        "system-token"
    );
    assert_eq!(
        grants.calls(),
        vec!["ManagedIdentityCredential:system".to_owned()]
    );
}

fn environment_token_runs_first() {
    let grants =
        FixtureGrants::new().script(CLIENT_SECRET, Script::Token("secret-token".to_owned()));
    let env = environment(&[
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_CLIENT_SECRET", "env-secret"),
        ("AZURE_TENANT_ID", "env-tenant"),
    ]);
    let token_provider = provider(None, Some(DEFAULT), &env, &grants).unwrap();
    assert_eq!(grants.records()[0].credential, CLIENT_SECRET);
    assert_eq!(
        token_provider.token().unwrap().token.expose_secret(),
        "secret-token"
    );
    assert_eq!(grants.calls(), vec![CLIENT_SECRET.to_owned()]);
}

fn empty_values_still_select_environment() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CLIENT_ID", ""),
        ("AZURE_CLIENT_SECRET", ""),
        ("AZURE_TENANT_ID", ""),
    ]);
    assert_eq!(
        env.infer_credential_type(),
        AzureCredentialType::DefaultAzureCredential
    );
    let token_provider = provider(None, Some(DEFAULT), &env, &grants).unwrap();
    assert_eq!(token_provider.credential, DEFAULT);
    assert_eq!(grants.records()[0].credential, CLIENT_SECRET);
    assert_eq!(grants.records()[0].client_id.as_deref(), Some(""));
}

fn authentication_error_stops_the_chain() {
    let grants = FixtureGrants::new().script(
        CLIENT_SECRET,
        Script::Authentication("bad secret".to_owned()),
    );
    let env = environment(&[
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_CLIENT_SECRET", "env-secret"),
        ("AZURE_TENANT_ID", "env-tenant"),
    ]);
    let token_provider = provider(None, Some(DEFAULT), &env, &grants).unwrap();
    let error = token_provider.token().unwrap_err();
    assert_eq!(error.category(), "client_authentication");
    let message = error.to_string();
    assert!(message.starts_with("DefaultAzureCredential failed to retrieve a token"));
    assert!(message.contains("\tEnvironmentCredential: bad secret"));
    assert!(!message.contains("ManagedIdentityCredential"));
    assert_eq!(grants.calls(), vec![CLIENT_SECRET.to_owned()]);
}

pub(super) fn credential_constructor_arguments() {
    client_secret_arguments();
    managed_identity_arguments();
    certificate_arguments();
}

fn client_secret_arguments() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", CLIENT_SECRET),
        ("AZURE_CLIENT_ID", "argument-client"),
        ("AZURE_CLIENT_SECRET", "argument-secret"),
        ("AZURE_TENANT_ID", "argument-tenant"),
    ]);
    provider(None, None, &env, &grants).unwrap();
    let record = &grants.records()[0];
    assert_eq!(record.client_id.as_deref(), Some("argument-client"));
    assert_eq!(record.client_secret.as_deref(), Some("argument-secret"));
    assert_eq!(record.tenant_id.as_deref(), Some("argument-tenant"));
    missing_secret_and_tenant();
}

fn missing_secret_and_tenant() {
    let grants = FixtureGrants::new();
    let missing_secret = environment(&[
        ("AZURE_CREDENTIAL", CLIENT_SECRET),
        ("AZURE_CLIENT_ID", "argument-client"),
    ]);
    assert_eq!(
        provider(None, None, &missing_secret, &grants)
            .unwrap_err()
            .detail()
            .0,
        Some("AZURE_CLIENT_SECRET")
    );
    let missing_tenant = environment(&[
        ("AZURE_CREDENTIAL", CLIENT_SECRET),
        ("AZURE_CLIENT_ID", "argument-client"),
        ("AZURE_CLIENT_SECRET", "argument-secret"),
    ]);
    assert_eq!(
        provider(None, None, &missing_tenant, &grants)
            .unwrap_err()
            .detail()
            .0,
        Some("AZURE_TENANT_ID")
    );
    let missing_client = environment(&[("AZURE_CREDENTIAL", CLIENT_SECRET)]);
    assert_eq!(
        provider(None, None, &missing_client, &grants)
            .unwrap_err()
            .detail()
            .0,
        Some("AZURE_CLIENT_ID")
    );
}

fn managed_identity_arguments() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", MANAGED),
        ("AZURE_CLIENT_ID", "identity-client"),
    ]);
    provider(None, None, &env, &grants).unwrap();
    assert_eq!(
        grants.records()[0].client_id.as_deref(),
        Some("identity-client")
    );
    let missing = environment(&[("AZURE_CREDENTIAL", MANAGED)]);
    assert_eq!(
        provider(None, None, &missing, &grants)
            .unwrap_err()
            .detail()
            .0,
        Some("AZURE_CLIENT_ID")
    );
}

fn certificate_arguments() {
    let grants = FixtureGrants::new();
    let with_password = [
        ("AZURE_CREDENTIAL", CERTIFICATE),
        ("AZURE_CLIENT_ID", "cert-client"),
        ("AZURE_TENANT_ID", "cert-tenant"),
        ("AZURE_CERTIFICATE_PATH", "/certs/azure.pem"),
        ("AZURE_CERTIFICATE_PASSWORD", "cert-password"),
    ];
    let env = environment(&with_password);
    provider(None, None, &env, &grants).unwrap();
    assert_eq!(
        grants.records()[0].password.as_deref(),
        Some("cert-password")
    );
    let without_password = [
        ("AZURE_CREDENTIAL", CERTIFICATE),
        ("AZURE_CLIENT_ID", "cert-client"),
        ("AZURE_TENANT_ID", "cert-tenant"),
        ("AZURE_CERTIFICATE_PATH", "/certs/azure.pem"),
    ];
    let without_password_environment = environment(&without_password);
    provider(None, None, &without_password_environment, &grants).unwrap();
    assert_eq!(grants.records()[1].password, None);
    empty_password_is_not_forwarded();
    missing_certificate_path();
}

fn empty_password_is_not_forwarded() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", CERTIFICATE),
        ("AZURE_CLIENT_ID", "cert-client"),
        ("AZURE_TENANT_ID", "cert-tenant"),
        ("AZURE_CERTIFICATE_PATH", "/certs/azure.pem"),
        ("AZURE_CERTIFICATE_PASSWORD", ""),
    ]);
    provider(None, None, &env, &grants).unwrap();
    assert_eq!(grants.records()[0].password, None);
    assert!(grants.calls().is_empty());
}

fn missing_certificate_path() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", CERTIFICATE),
        ("AZURE_CLIENT_ID", "cert-client"),
        ("AZURE_TENANT_ID", "cert-tenant"),
    ]);
    assert_eq!(
        provider(None, None, &env, &grants).unwrap_err().detail().0,
        Some("AZURE_CERTIFICATE_PATH")
    );
}

pub(super) fn default_environment_certificate() {
    environment_certificate_record();
    send_certificate_chain_flag();
    environment_username_password();
    lite_llm_certificate_inputs_do_not_select_the_environment_link();
}

fn environment_certificate_record() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CLIENT_ID", "cert-client"),
        ("AZURE_TENANT_ID", "cert-tenant"),
        ("AZURE_CLIENT_CERTIFICATE_PATH", "/certs/client.pem"),
        ("AZURE_CLIENT_CERTIFICATE_PASSWORD", "client-cert-password"),
    ]);
    provider(None, Some(DEFAULT), &env, &grants).unwrap();
    let record = &grants.records()[0];
    assert_eq!(record.credential, CERTIFICATE);
    assert_eq!(
        record.certificate_path.as_deref(),
        Some("/certs/client.pem")
    );
    assert_eq!(record.password.as_deref(), Some("client-cert-password"));
    assert_eq!(record.send_certificate_chain, Some(false));
}

fn send_certificate_chain_flag() {
    assert_eq!(send_chain_for("false"), Some(true));
    assert_eq!(send_chain_for("0"), Some(true));
    assert_eq!(send_chain_for(""), Some(false));
}

fn send_chain_for(value: &str) -> Option<bool> {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CLIENT_ID", "cert-client"),
        ("AZURE_TENANT_ID", "cert-tenant"),
        ("AZURE_CLIENT_CERTIFICATE_PATH", "/certs/client.pem"),
        ("AZURE_CLIENT_SEND_CERTIFICATE_CHAIN", value),
    ]);
    provider(None, Some(DEFAULT), &env, &grants).unwrap();
    grants.records()[0].send_certificate_chain
}

fn environment_username_password() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CLIENT_ID", "password-client"),
        ("AZURE_USERNAME", "user@example.test"),
        ("AZURE_PASSWORD", "fixture-password"),
        ("AZURE_TENANT_ID", "password-tenant"),
    ]);
    let token_provider = provider(None, Some(DEFAULT), &env, &grants).unwrap();
    let record = &grants.records()[0];
    assert_eq!(record.credential, USERNAME_PASSWORD);
    assert_eq!(record.tenant_id.as_deref(), Some("password-tenant"));
    assert_eq!(record.username.as_deref(), Some("user@example.test"));
    assert_eq!(
        token_provider.warnings,
        vec![ENVIRONMENT_PASSWORD_WARNING.to_owned()]
    );
    let without_tenant = environment(&[
        ("AZURE_CLIENT_ID", "password-client"),
        ("AZURE_USERNAME", "user@example.test"),
        ("AZURE_PASSWORD", "fixture-password"),
    ]);
    provider(None, Some(DEFAULT), &without_tenant, &grants).unwrap();
    assert_eq!(grants.records()[1].tenant_id, None);
}

fn lite_llm_certificate_inputs_do_not_select_the_environment_link() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CLIENT_ID", "cert-client"),
        ("AZURE_TENANT_ID", "cert-tenant"),
        ("AZURE_CERTIFICATE_PATH", "/certs/azure.pem"),
        ("AZURE_CERTIFICATE_PASSWORD", "azure-cert-password"),
    ]);
    provider(None, Some(DEFAULT), &env, &grants).unwrap();
    assert_eq!(grants.records()[0].credential, MANAGED);
    assert!(
        grants
            .records()
            .iter()
            .all(|record| record.credential != CERTIFICATE)
    );
}

pub(super) fn default_credential_filter() {
    filter_selection();
    managed_identity_probe_flag();
    invalid_filter_message();
    developer_filter_is_unsupported();
}

fn filter_records(token_credentials: &str) -> Vec<String> {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CLIENT_ID", "filter-client"),
        ("AZURE_CLIENT_SECRET", "filter-secret"),
        ("AZURE_TENANT_ID", "filter-tenant"),
        ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ("AZURE_TOKEN_CREDENTIALS", token_credentials),
    ]);
    provider(None, Some(DEFAULT), &env, &grants).unwrap();
    grants
        .records()
        .into_iter()
        .map(|record| record.credential)
        .collect()
}

fn filter_selection() {
    let all = vec![
        CLIENT_SECRET.to_owned(),
        WORKLOAD.to_owned(),
        MANAGED.to_owned(),
    ];
    assert_eq!(filter_records(""), all);
    assert_eq!(filter_records("prod"), all);
    assert_eq!(
        filter_records(" EnvironmentCredential "),
        vec![CLIENT_SECRET.to_owned()]
    );
    assert_eq!(
        filter_records("workloadidentitycredential"),
        vec![WORKLOAD.to_owned()]
    );
    assert_eq!(
        filter_records("MANAGEDIDENTITYCREDENTIAL"),
        vec![MANAGED.to_owned()]
    );
}

fn managed_identity_probe_flag() {
    let grants = FixtureGrants::new();
    let env = environment(&[("AZURE_TOKEN_CREDENTIALS", "managedidentitycredential")]);
    provider(None, Some(DEFAULT), &env, &grants).unwrap();
    let record = &grants.records()[0];
    assert_eq!(record.exclude_workload_identity, Some(true));
    assert_eq!(record.imds_probe, Some(false));
    assert_eq!(record.within_credential_chain, Some(true));
    assert_eq!(record.within_default_credential, Some(true));
}

fn invalid_filter_message() {
    let env = environment(&[("AZURE_TOKEN_CREDENTIALS", "bogus")]);
    let error = provider(None, Some(DEFAULT), &env, &FixtureGrants::new()).unwrap_err();
    assert_eq!(error.category(), "invalid_value");
    assert_eq!(
        error.to_string(),
        "Invalid value for AZURE_TOKEN_CREDENTIALS: bogus. Valid values are: dev, prod, azureclicredential, azuredeveloperclicredential, azurepowershellcredential, environmentcredential, interactivebrowsercredential, managedidentitycredential, visualstudiocodecredential, workloadidentitycredential."
    );
}

fn developer_filter_is_unsupported() {
    assert_eq!(selection("dev").unwrap_err().category(), "unsupported");
}

pub(super) fn default_provider_try_boundary() {
    successful_boundary();
    construction_error_is_caught();
    invalid_filter_is_caught();
    invocation_error_is_not_caught();
}

fn successful_boundary() {
    let grants = FixtureGrants::new().script(
        "ManagedIdentityCredential:system",
        Script::Token("system-token".to_owned()),
    );
    let env = environment(&[]);
    let boundary = try_default_credential("explicit-scope", &env, &grants);
    let token_provider = boundary.provider.unwrap();
    assert!(boundary.failure.is_none());
    assert_eq!(token_provider.scope, "explicit-scope");
    assert!(grants.calls().is_empty());
    assert_eq!(
        token_provider.token().unwrap().token.expose_secret(),
        "system-token"
    );
}

fn construction_error_is_caught() {
    let grants =
        FixtureGrants::new().construct_error(CLIENT_SECRET, "fixture construction failure");
    let env = environment(&[
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_CLIENT_SECRET", "env-secret"),
        ("AZURE_TENANT_ID", "env-tenant"),
    ]);
    let boundary = try_default_credential("scope", &env, &grants);
    assert!(boundary.provider.is_none());
    assert_eq!(boundary.failure.unwrap().category(), "construction");
    assert!(grants.calls().is_empty());
}

fn invalid_filter_is_caught() {
    let grants = FixtureGrants::new();
    let env = environment(&[("AZURE_TOKEN_CREDENTIALS", "bogus")]);
    let boundary = try_default_credential("scope", &env, &grants);
    assert!(boundary.provider.is_none());
    assert_eq!(boundary.failure.unwrap().category(), "invalid_value");
}

fn invocation_error_is_not_caught() {
    let grants = FixtureGrants::new().script(
        "ManagedIdentityCredential:system",
        Script::Authentication("identity denied".to_owned()),
    );
    let env = environment(&[]);
    let boundary = try_default_credential("scope", &env, &grants);
    let error = boundary.provider.unwrap().token().unwrap_err();
    assert_eq!(error.category(), "client_authentication");
    assert!(error.to_string().contains("identity denied"));
}

pub(super) fn chain_exhaustion() {
    default_credential_exhaustion();
    environment_short_circuit_message();
    deployment_exhaustion();
}

fn default_credential_exhaustion() {
    let grants = FixtureGrants::new().script(
        "ManagedIdentityCredential:system",
        Script::Unavailable("imds unavailable".to_owned()),
    );
    let token_provider = default_provider(&[], &grants);
    let error = token_provider.token().unwrap_err();
    let expected = format!(
        "DefaultAzureCredential failed to retrieve a token from the included credentials.\nAttempted credentials:\n\tEnvironmentCredential: {ENVIRONMENT_UNAVAILABLE}\n\tWorkloadIdentityCredential: WorkloadIdentityCredential authentication unavailable. The workload options are not fully configured. See the troubleshooting guide for more information: https://aka.ms/azsdk/python/identity/workloadidentitycredential/troubleshoot. Missing required arguments: 'tenant_id', 'client_id', 'token_file_path'.\n\tManagedIdentityCredential: imds unavailable\nTo mitigate this issue, please refer to the troubleshooting guidelines here at {TROUBLESHOOT_URL}."
    );
    assert_eq!(error.to_string(), expected);
    assert_eq!(error.category(), "client_authentication");
}

fn environment_short_circuit_message() {
    let grants = FixtureGrants::new().script(
        CLIENT_SECRET,
        Script::Authentication("bad secret".to_owned()),
    );
    let env = environment(&[
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_CLIENT_SECRET", "env-secret"),
        ("AZURE_TENANT_ID", "env-tenant"),
    ]);
    let token_provider = provider(None, Some(DEFAULT), &env, &grants).unwrap();
    let expected = format!(
        "DefaultAzureCredential failed to retrieve a token from the included credentials.\nAttempted credentials:\n\tEnvironmentCredential: bad secret\nTo mitigate this issue, please refer to the troubleshooting guidelines here at {TROUBLESHOOT_URL}."
    );
    assert_eq!(token_provider.token().unwrap_err().to_string(), expected);
}

fn deployment_exhaustion() {
    let grants = FixtureGrants::new()
        .script(WORKLOAD, Script::Unavailable("workload missing".to_owned()))
        .script(
            "ManagedIdentityCredential:user",
            Script::Unavailable("user identity missing".to_owned()),
        )
        .script(
            "ManagedIdentityCredential:system",
            Script::Unavailable("no system identity".to_owned()),
        );
    let token_provider = deployment_provider(
        &[
            ("AZURE_CLIENT_ID", "pod-client"),
            ("AZURE_TENANT_ID", "pod-tenant"),
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ],
        &grants,
    );
    let expected = format!(
        "ChainedTokenCredential failed to retrieve a token from the included credentials.\nAttempted credentials:\n\tWorkloadIdentityCredential: workload missing\n\tManagedIdentityCredential: user identity missing\n\tManagedIdentityCredential: no system identity\nTo mitigate this issue, please refer to the troubleshooting guidelines here at {TROUBLESHOOT_URL}."
    );
    assert_eq!(token_provider.token().unwrap_err().to_string(), expected);
}

pub(super) fn generic_credential_constructor() {
    workload_environment_constructor();
    workload_missing_arguments();
    username_password_without_arguments();
}

fn workload_environment_constructor() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", WORKLOAD),
        ("AZURE_CLIENT_ID", "workload-client"),
        ("AZURE_TENANT_ID", "workload-tenant"),
        ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
    ]);
    let token_provider = provider(None, None, &env, &grants).unwrap();
    assert_eq!(
        token_provider.credential,
        AzureCredentialType::WorkloadIdentityCredential
    );
    let record = &grants.records()[0];
    assert_eq!(record.credential, WORKLOAD);
    assert_eq!(record.client_id.as_deref(), Some("workload-client"));
    assert_eq!(record.token_file.as_deref(), Some("/var/run/token"));
    assert!(grants.calls().is_empty());
}

fn workload_missing_arguments() {
    let env = environment(&[
        ("AZURE_CREDENTIAL", WORKLOAD),
        ("AZURE_CLIENT_ID", "workload-client"),
    ]);
    let error = provider(None, None, &env, &FixtureGrants::new()).unwrap_err();
    assert_eq!(error.category(), "invalid_value");
    assert_eq!(
        error.to_string(),
        "WorkloadIdentityCredential authentication unavailable. The workload options are not fully configured. See the troubleshooting guide for more information: https://aka.ms/azsdk/python/identity/workloadidentitycredential/troubleshoot. Missing required arguments: 'tenant_id', 'token_file_path'."
    );
}

fn username_password_without_arguments() {
    let grants = FixtureGrants::new();
    let env = environment(&[
        ("AZURE_CREDENTIAL", USERNAME_PASSWORD),
        ("AZURE_CLIENT_ID", "password-client"),
        ("AZURE_USERNAME", "user@example.test"),
        ("AZURE_PASSWORD", "fixture-password"),
    ]);
    let error = provider(None, None, &env, &grants).unwrap_err();
    assert_eq!(error.category(), "type_error");
    assert_eq!(
        error.detail().1,
        Some(["client_id", "username", "password"].as_slice())
    );
    assert!(grants.records().is_empty());
}

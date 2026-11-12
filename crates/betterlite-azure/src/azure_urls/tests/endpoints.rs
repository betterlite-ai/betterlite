use betterlite_config::sections::azure_url_policy::AzureCredentials;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_urls::{
    AuthParams, AzureUrlError, EndpointSelection, get_azure_v1_image_url, get_base_azure_url,
    resolve_auth_environment, select_base_url_or_endpoint,
};

use super::support;

#[rstest]
#[case::credential_precedence(0)]
#[case::deployment_base_url(1)]
#[case::v1_latest_preview(2)]
#[case::query_api_version_wins(3)]
#[case::append_route_once(4)]
#[case::azure_header_selection(5)]
#[case::azure_env_none_vs_empty(6)]
#[case::v1_image_resource_reset(7)]
fn endpoints_cases(#[case] case: u8) {
    match case {
        0 => credential_precedence(),
        1 => deployment_base_url(),
        2 => v1_latest_preview(),
        3 => query_api_version_wins(),
        4 => append_route_once(),
        5 => azure_header_selection(),
        6 => azure_env_none_vs_empty(),
        _ => v1_image_resource_reset(),
    }
}

fn secret(credentials: &AzureCredentials) -> Option<String> {
    credentials
        .api_key
        .as_ref()
        .map(|value| value.expose_secret().to_owned())
}

const ENVIRONMENT: [(&str, &str); 3] = [
    ("AZURE_API_BASE", "https://environment.example"),
    ("AZURE_API_VERSION", "environment-version"),
    ("AZURE_OPENAI_API_KEY", "environment-key"),
];

fn credential_precedence() {
    let empty = support::credentials("", &[]);
    assert_eq!(empty.api_base, None);
    assert_eq!(empty.api_version, None);
    assert_eq!(secret(&empty), None);
    deployment_credentials_win();
    module_and_environment_credentials_win();
    blank_explicit_values_fall_through();
}

fn deployment_credentials_win() {
    let deployment = support::credentials(
        "      api_base: https://deployment.example\n      api_version: deployment-version\n      api_key: deployment-key\n",
        &ENVIRONMENT,
    );
    assert_eq!(
        deployment.api_base.as_deref(),
        Some("https://deployment.example")
    );
    assert_eq!(
        deployment.api_version.as_deref(),
        Some("deployment-version")
    );
    assert_eq!(secret(&deployment).as_deref(), Some("deployment-key"));
}

fn module_and_environment_credentials_win() {
    let module = support::credentials(
        "litellm_settings:\n  api_base: https://module.example\n  api_version: module-version\n  api_key: module-key\n",
        &ENVIRONMENT,
    );
    assert_eq!(module.api_base.as_deref(), Some("https://module.example"));
    assert_eq!(module.api_version.as_deref(), Some("module-version"));
    assert_eq!(secret(&module).as_deref(), Some("module-key"));
    let azure_key = support::credentials(
        "litellm_settings:\n  azure_key: azure-key\n",
        &[("AZURE_OPENAI_API_KEY", "environment-key")],
    );
    assert_eq!(secret(&azure_key).as_deref(), Some("azure-key"));
    let environment = support::credentials("", &ENVIRONMENT);
    assert_eq!(
        environment.api_base.as_deref(),
        Some("https://environment.example")
    );
    assert_eq!(
        environment.api_version.as_deref(),
        Some("environment-version")
    );
    assert_eq!(secret(&environment).as_deref(), Some("environment-key"));
    let last_key = support::credentials("", &[("AZURE_API_KEY", "last-key")]);
    assert_eq!(secret(&last_key).as_deref(), Some("last-key"));
}

fn blank_explicit_values_fall_through() {
    let blank = support::credentials(
        "      api_base: \"\"\n      api_version: \"\"\n      api_key: \"\"\n",
        &[("AZURE_API_KEY", "last-key")],
    );
    assert_eq!(blank.api_base, None);
    assert_eq!(blank.api_version, None);
    assert_eq!(secret(&blank).as_deref(), Some("last-key"));
}

fn deployment_base_url() {
    moved_endpoint_becomes_base_url();
    other_values_stay();
}

fn moved_endpoint_becomes_base_url() {
    let moved = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: Some(
            "https://example.openai.azure.com/openai/deployments/deployment".to_owned(),
        ),
        base_url: Some("https://previous.example".to_owned()),
    });
    assert_eq!(moved.azure_endpoint, None);
    assert_eq!(
        moved.base_url.as_deref(),
        Some("https://example.openai.azure.com/openai/deployments/deployment")
    );
    let embedded = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: Some(
            "https://example.openai.azure.com/prefix/openai/deployments?x=1".to_owned(),
        ),
        base_url: None,
    });
    assert_eq!(
        embedded.base_url.as_deref(),
        Some("https://example.openai.azure.com/prefix/openai/deployments?x=1")
    );
    assert_eq!(embedded.azure_endpoint, None);
}

fn other_values_stay() {
    let unchanged = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: Some("https://example.openai.azure.com".to_owned()),
        base_url: Some("https://previous.example".to_owned()),
    });
    assert_eq!(
        unchanged.azure_endpoint.as_deref(),
        Some("https://example.openai.azure.com")
    );
    assert_eq!(
        unchanged.base_url.as_deref(),
        Some("https://previous.example")
    );
    let absent = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: None,
        base_url: Some("https://previous.example".to_owned()),
    });
    assert_eq!(absent.azure_endpoint, None);
    assert_eq!(absent.base_url.as_deref(), Some("https://previous.example"));
    let blank = select_base_url_or_endpoint(EndpointSelection {
        azure_endpoint: Some(String::new()),
        base_url: None,
    });
    assert_eq!(blank.azure_endpoint.as_deref(), Some(""));
    assert_eq!(blank.base_url, None);
}

fn v1_latest_preview() {
    v1_composes_the_v1_path();
    v1_uses_the_default_version();
}

fn v1_composes_the_v1_path() {
    let base = Some("https://example.openai.azure.com");
    assert_eq!(
        get_base_azure_url(base, Some("preview-2024-02-01"), None, "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/responses?api-version=preview-2024-02-01"
    );
    assert_eq!(
        get_base_azure_url(base, Some("Latest"), None, "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/responses?api-version=Latest"
    );
    assert_eq!(
        get_base_azure_url(base, Some("v1"), None, "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/v1/responses?api-version=v1"
    );
    assert_eq!(
        get_base_azure_url(base, Some("latest"), None, "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/v1/responses?api-version=latest"
    );
    assert_eq!(
        get_base_azure_url(base, Some("preview"), None, "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/v1/responses?api-version=preview"
    );
    assert_eq!(
        get_base_azure_url(base, Some("V1"), None, "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/responses?api-version=V1"
    );
    assert_eq!(
        get_base_azure_url(base, None, None, "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/responses"
    );
}

fn v1_uses_the_default_version() {
    let base = Some("https://example.openai.azure.com");
    assert_eq!(
        get_base_azure_url(base, None, Some("v1"), "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/v1/responses?api-version=v1"
    );
    assert_eq!(
        get_base_azure_url(base, Some(""), Some("v1"), "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/v1/responses?api-version=v1"
    );
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com/openai/v1"),
            Some("v1"),
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/openai/v1/openai/responses?api-version=v1"
    );
    // an appended route re-encodes the query, so a query holding /openai/v1 does not stop the rewrite
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com?target=/openai/v1&keep=1"),
            Some("v1"),
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/openai/v1/responses?target=%2Fopenai%2Fv1&keep=1&api-version=v1"
    );
    // an api_base the append skips is checked as written, so its query can suppress the rewrite
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com/api?target=/openai/responses&x=/openai/v1"),
            Some("v1"),
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/api?target=%2Fopenai%2Fresponses&x=%2Fopenai%2Fv1&api-version=v1"
    );
}

fn query_api_version_wins() {
    existing_query_version_wins();
    missing_query_version_is_added();
    unresolved_base_is_an_error();
    resolved_bases_come_from_config();
}

fn existing_query_version_wins() {
    let existing = "https://example.openai.azure.com/openai/responses?api-version=2024-02-01";
    assert_eq!(
        get_base_azure_url(
            Some(existing),
            Some("2025-01-01"),
            None,
            "/openai/responses"
        )
        .unwrap(),
        existing
    );
    let blank = "https://example.openai.azure.com/openai/responses?api-version=";
    assert_eq!(
        get_base_azure_url(Some(blank), Some("2025-01-01"), None, "/openai/responses").unwrap(),
        blank
    );
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com/openai/responses?api-version=first&api-version=second"),
            Some("2025-01-01"),
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/openai/responses?api-version=first"
    );
}

fn missing_query_version_is_added() {
    let other = "https://example.openai.azure.com/openai/responses?x=1&y=2";
    assert_eq!(
        get_base_azure_url(Some(other), Some("2025-01-01"), None, "/openai/responses").unwrap(),
        "https://example.openai.azure.com/openai/responses?x=1&y=2&api-version=2025-01-01"
    );
    assert_eq!(
        get_base_azure_url(Some(other), None, None, "/openai/responses").unwrap(),
        other
    );
}

fn unresolved_base_is_an_error() {
    assert!(matches!(
        get_base_azure_url(None, None, None, "/openai/responses").unwrap_err(),
        AzureUrlError::MissingApiBase
    ));
}

fn resolved_bases_come_from_config() {
    let environment =
        support::credentials("", &[("AZURE_API_BASE", "https://environment.example")]);
    assert_eq!(
        environment.api_base.as_deref(),
        Some("https://environment.example")
    );
    assert_eq!(
        get_base_azure_url(
            environment.api_base.as_deref(),
            Some("2025-01-01"),
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://environment.example/openai/responses?api-version=2025-01-01"
    );
    let module = support::credentials(
        "litellm_settings:\n  api_base: https://module.example\n",
        &[("AZURE_API_BASE", "https://environment.example")],
    );
    assert_eq!(module.api_base.as_deref(), Some("https://module.example"));
}

fn append_route_once() {
    route_presence_skips_append();
    route_presence_empty_path_edges();
    route_segments_merge();
}

fn route_presence_skips_append() {
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com/openai/responses"),
            None,
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/openai/responses"
    );
    // a route that appears anywhere in the URL, the query included, suppresses the append
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com?target=/openai/responses"),
            None,
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com?target=%2Fopenai%2Fresponses"
    );
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com/api?target=/openai/responses"),
            None,
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/api?target=%2Fopenai%2Fresponses"
    );
}

fn route_presence_empty_path_edges() {
    // an explicit empty path keeps its slash, unlike a bare authority
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com/?target=/openai/responses"),
            None,
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/?target=%2Fopenai%2Fresponses"
    );
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com?target=/openai/responses"),
            Some("v1"),
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/?target=%2Fopenai%2Fresponses&api-version=v1"
    );
}

fn route_segments_merge() {
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com/openai"),
            None,
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/openai/responses"
    );
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com/api"),
            None,
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/api/openai/responses"
    );
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com?a=1&a=2"),
            None,
            None,
            "/r",
        )
        .unwrap(),
        "https://example.openai.azure.com/r?a=1"
    );
    assert_eq!(
        get_base_azure_url(
            Some("https://example.openai.azure.com?api-version=1&q=a"),
            Some("v1"),
            None,
            "/openai/responses",
        )
        .unwrap(),
        "https://example.openai.azure.com/openai/v1/responses?api-version=1&q=a"
    );
}

fn azure_header_selection() {
    let key = SecretString::from("fixture-key");
    let empty = support::token_settings("", &[]);
    let token = support::token_settings("      azure_ad_token: fixture-token\n", &[]);
    let blank_token = support::token_settings("      azure_ad_token: \"\"\n", &[]);
    let kept = support::environment_headers(&[("api-key", "")], Some(&key), &token).unwrap();
    assert_eq!(kept.get("api-key").map(String::as_str), Some(""));
    assert_eq!(kept.len(), 1);
    let added = support::environment_headers(&[], Some(&key), &empty).unwrap();
    assert_eq!(
        added.get("api-key").map(String::as_str),
        Some("fixture-key")
    );
    let cased =
        support::environment_headers(&[("API-Key", "previous")], Some(&key), &empty).unwrap();
    assert_eq!(cased.get("API-Key").map(String::as_str), Some("previous"));
    assert_eq!(
        cased.get("api-key").map(String::as_str),
        Some("fixture-key")
    );
    let bearer = support::environment_headers(&[("API-Key", "previous")], None, &token).unwrap();
    assert_eq!(
        bearer.get("Authorization").map(String::as_str),
        Some("Bearer fixture-token")
    );
    assert_eq!(bearer.get("API-Key").map(String::as_str), Some("previous"));
    assert!(
        support::environment_headers(&[], None, &blank_token)
            .unwrap()
            .is_empty()
    );
    assert!(
        support::environment_headers(&[], None, &empty)
            .unwrap()
            .is_empty()
    );
}

fn azure_env_none_vs_empty() {
    let keys = [
        ("tenant_id", "AZURE_TENANT_ID"),
        ("client_id", "AZURE_CLIENT_ID"),
        ("client_secret", "AZURE_CLIENT_SECRET"),
        ("azure_username", "AZURE_USERNAME"),
        ("azure_password", "AZURE_PASSWORD"),
        ("azure_scope", "AZURE_SCOPE"),
    ];
    for (param, variable) in keys {
        check_env_key(param, variable);
    }
}

fn check_env_key(param: &str, variable: &str) {
    let environment = [(variable, "environment")];
    assert_eq!(
        resolve(Some((param, Some("explicit"))), &environment, param).as_deref(),
        Some("explicit")
    );
    assert_eq!(
        resolve(Some((param, Some(""))), &environment, param).as_deref(),
        Some("")
    );
    assert_eq!(
        resolve(Some((param, None)), &environment, param).as_deref(),
        Some("environment")
    );
    assert_eq!(
        resolve(None, &environment, param).as_deref(),
        Some("environment")
    );
    assert_eq!(resolve(None, &[], param), None);
}

fn params_with(key: &str, value: Option<&str>) -> AuthParams {
    let mut params = AuthParams::default();
    match key {
        "tenant_id" => params.tenant_id = value.map(str::to_owned),
        "client_id" => params.client_id = value.map(str::to_owned),
        "client_secret" => params.client_secret = value.map(SecretString::from),
        "azure_username" => params.azure_username = value.map(str::to_owned),
        "azure_password" => params.azure_password = value.map(SecretString::from),
        "azure_scope" => params.azure_scope = value.map(str::to_owned),
        other => panic!("unknown auth param {other}"),
    }
    params
}

fn resolve(
    param: Option<(&str, Option<&str>)>,
    environment: &[(&str, &str)],
    key: &str,
) -> Option<String> {
    let params = match param {
        Some((name, value)) => params_with(name, value),
        None => AuthParams::default(),
    };
    let resolved = resolve_auth_environment(params, &support::environment_map(environment));
    match key {
        "tenant_id" => resolved.tenant_id,
        "client_id" => resolved.client_id,
        "client_secret" => resolved
            .client_secret
            .map(|secret| secret.expose_secret().to_owned()),
        "azure_username" => resolved.azure_username,
        "azure_password" => resolved
            .azure_password
            .map(|secret| secret.expose_secret().to_owned()),
        "azure_scope" => resolved.azure_scope,
        other => panic!("unknown auth param {other}"),
    }
}

fn v1_image_resource_reset() {
    image_dated_versions_are_unchanged();
    image_v1_versions_reset_the_resource();
}

fn image_dated_versions_are_unchanged() {
    let deployment =
        "https://example.openai.azure.com/openai/deployments/deployment/images/generations";
    assert_eq!(
        get_azure_v1_image_url(
            &format!("{deployment}?api-version=2024-02-01"),
            Some("2024-02-01"),
            "/openai/v1/images/generations",
        )
        .unwrap(),
        None
    );
    assert_eq!(
        get_azure_v1_image_url(deployment, None, "/openai/v1/images/generations").unwrap(),
        None
    );
}

fn image_v1_versions_reset_the_resource() {
    let deployment =
        "https://example.openai.azure.com/openai/deployments/deployment/images/generations";
    assert_image_url(
        &format!("{deployment}?api-version=2024-02-01"),
        Some("v1"),
        Some("https://example.openai.azure.com/openai/v1/images/generations?api-version=v1"),
    );
    assert_image_url(
        &format!("{deployment}?keep=1&api-version=2024-02-01&api-version=2024-03-01&z=2"),
        Some("preview"),
        Some(
            "https://example.openai.azure.com/openai/v1/images/generations?keep=1&z=2&api-version=preview",
        ),
    );
    assert_image_url(
        "https://example.openai.azure.com/prefix/openai/deployments?api-version=1&keep=1",
        Some("latest"),
        Some(
            "https://example.openai.azure.com/prefix/openai/v1/images/generations?keep=1&api-version=latest",
        ),
    );
    assert_image_url(
        "https://example.openai.azure.com/images",
        Some("v1"),
        Some("https://example.openai.azure.com/images/openai/v1/images/generations?api-version=v1"),
    );
}

fn assert_image_url(api_base: &str, api_version: Option<&str>, expected: Option<&str>) {
    let image_route = "/openai/v1/images/generations";
    assert_eq!(
        get_azure_v1_image_url(api_base, api_version, image_route)
            .unwrap()
            .as_deref(),
        expected
    );
}

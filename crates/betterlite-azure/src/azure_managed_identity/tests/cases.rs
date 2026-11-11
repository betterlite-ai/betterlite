use std::iter::repeat_n;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::azure_hosted_identity::protocol::HostedProtocol;

use super::super::error::{NO_ENDPOINT, NO_RESPONSE_PREFIX};
use super::super::imds::{CLIENT_SKU, CLIENT_VERSION, ProbePolicy};
use super::super::selection::ManagedIdentityKind;
use super::fixture::{APP_SERVICE, Fixture, NOW, REQUEST_ID, Reply};

const SCOPE: &str = "https://cognitiveservices.azure.com/.default";
const RESOURCE: &str = "https://cognitiveservices.azure.com";
const THUMBPRINT: &str = "00112233445566778899aabbccddeeff00112233";
const TOKEN: &str = r#"{"access_token":"imds-token","expires_in":3600}"#;

#[rstest]
#[case::system_assigned_imds(system_assigned_imds)]
#[case::client_id_query(client_id_query)]
#[case::imds_unavailable(imds_unavailable)]
#[case::token_expiry(token_expiry)]
#[case::workload_selection(workload_selection)]
#[case::imds_retry_policy(imds_retry_policy)]
#[case::hosted_selection_order(hosted_selection_order)]
#[case::partial_identity_endpoint(partial_identity_endpoint)]
fn azure_managed_identity_cases(#[case] case: fn()) {
    case();
}

fn system_assigned_imds() {
    let fixture = Fixture::new(&[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://pod.test/")]);
    fixture.imds.push(Reply::ok(TOKEN));
    let token = fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.token.expose_secret(), "imds-token");
    assert_eq!(token.expires_on, Some(NOW + 3600));
    assert_eq!(token.token_type.as_deref(), Some("Bearer"));
    assert_eq!(token.refresh_on, None);
    let requests = fixture.imds.recorded();
    assert_eq!(requests.len(), 1);
    imds_request_shape(&requests[0]);
    default_authority();
}

fn imds_request_shape(request: &super::fixture::RecordedRequest) {
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.url,
        "http://pod.test/metadata/identity/oauth2/token"
    );
    assert_eq!(request.query("api-version"), Some("2018-02-01"));
    assert_eq!(request.query("resource"), Some(RESOURCE));
    assert_eq!(request.query("client_id"), None);
    assert_eq!(request.query.len(), 2);
    assert_eq!(request.header("metadata"), Some("true"));
    assert_eq!(request.header("x-client-sku"), Some(CLIENT_SKU));
    assert_eq!(request.header("x-client-ver"), Some(CLIENT_VERSION));
    assert_eq!(request.header("x-ms-client-request-id"), Some(REQUEST_ID));
    assert_eq!(request.timeout_seconds, None);
}

fn default_authority() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::ok(TOKEN));
    fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(
        fixture.imds.recorded()[0].url,
        "http://169.254.169.254/metadata/identity/oauth2/token"
    );
}

fn client_id_query() {
    let user_assigned = Fixture::new(&[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://pod.test")]);
    user_assigned.imds.push(Reply::ok(TOKEN));
    user_assigned
        .credential(Some("fixture-client"))
        .token(&user_assigned.io(), &[SCOPE])
        .unwrap();
    let requests = user_assigned.imds.recorded();
    assert_eq!(requests[0].query("client_id"), Some("fixture-client"));
    assert_eq!(requests[0].query("object_id"), None);
    assert_eq!(requests[0].query("resource_id"), None);

    let system_assigned = Fixture::new(&[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://pod.test")]);
    system_assigned.imds.push(Reply::ok(TOKEN));
    system_assigned
        .credential(Some(""))
        .token(&system_assigned.io(), &[SCOPE])
        .unwrap();
    assert_eq!(system_assigned.imds.recorded()[0].query("client_id"), None);
}

fn imds_unavailable() {
    probe_in_chain();
    probe_no_response();
    probe_forbidden_unreachable();
    probe_forbidden_other();
    status_400();
    status_500();
    malformed_text();
    pod_authority_skips_probe();
}

fn chained_probe(
    fixture: &Fixture,
) -> Result<super::super::ManagedIdentityToken, super::super::error::ManagedIdentityError> {
    let credential = super::super::ManagedIdentityCredential {
        environment: &fixture.environment,
        host: &fixture.host,
        client_id: None,
        exclude_workload_identity: false,
        within_default_credential: false,
        probe: ProbePolicy {
            enable: None,
            within_credential_chain: true,
        },
    };
    credential.token(&fixture.io(), &[SCOPE])
}

fn probe_in_chain() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::ok(TOKEN));
    fixture.imds.push(Reply::ok(TOKEN));
    chained_probe(&fixture).unwrap();
    let requests = fixture.imds.recorded();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].headers.len(), 0);
    assert_eq!(requests[0].timeout_seconds, Some(1));
    assert_eq!(requests[1].header("metadata"), Some("true"));
    assert_eq!(requests[1].timeout_seconds, None);
    assert!(fixture.scheduler.recorded().is_empty());
}

fn probe_no_response() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::Failure);
    let error = chained_probe(&fixture).unwrap_err();
    assert_eq!(fixture.imds.recorded().len(), 1);
    assert_eq!(error.to_string(), format!("{NO_RESPONSE_PREFIX} "));
}

fn probe_forbidden_unreachable() {
    let fixture = Fixture::new(&[]);
    fixture
        .imds
        .push(Reply::json(403, r#"{"error":"unreachable"}"#));
    let error = chained_probe(&fixture).unwrap_err();
    assert_eq!(fixture.imds.recorded().len(), 1);
    assert!(error.to_string().contains("unreachable"));
    assert!(
        error
            .to_string()
            .starts_with("ManagedIdentityCredential authentication unavailable. Error:")
    );
}

fn probe_forbidden_other() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::json(403, r#"{"error":"denied"}"#));
    fixture.imds.push(Reply::ok(TOKEN));
    chained_probe(&fixture).unwrap();
    assert_eq!(fixture.imds.recorded().len(), 2);
}

fn status_400() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::bare(400, "boom"));
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "ManagedIdentityCredential authentication unavailable. No identity has been assigned to this resource."
    );
    assert_eq!(fixture.imds.recorded().len(), 1);
}

fn status_500() {
    let fixture = Fixture::new(&[]);
    fixture
        .imds
        .push_all(repeat_n(Reply::json(500, "boom"), 6).collect());
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    assert!(matches!(
        error,
        super::super::error::ManagedIdentityError::ClientAuthentication { .. }
    ));
    assert_eq!(fixture.imds.recorded().len(), 6);
    assert_eq!(
        fixture.scheduler.recorded(),
        vec![
            Duration::from_millis(1600),
            Duration::from_millis(3200),
            Duration::from_millis(6400),
            Duration::from_millis(12800),
        ]
    );
}

fn malformed_text() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::text(200, "not-json"));
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "ManagedIdentityCredential authentication unavailable."
    );
}

fn pod_authority_skips_probe() {
    let fixture = Fixture::new(&[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "")]);
    fixture.imds.push(Reply::ok(TOKEN));
    chained_probe(&fixture).unwrap();
    let requests = fixture.imds.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].header("metadata"), Some("true"));
    assert_eq!(requests[0].url, "/metadata/identity/oauth2/token");
}

fn token_expiry() {
    token_expiry_success();
    token_expiry_failures();
}

fn token_expiry_success() {
    let full = Fixture::new(&[]);
    full.imds.push(Reply::ok(
        r#"{"access_token":"tok","expires_in":7200,"token_type":"Custom"}"#,
    ));
    let token = full.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.expires_on, Some(NOW + 7200));
    assert_eq!(token.token_type.as_deref(), Some("Custom"));
    assert_eq!(token.refresh_on, Some(NOW + 3600));

    let numeric = Fixture::new(&[]);
    numeric
        .imds
        .push(Reply::ok(r#"{"access_token":"tok","expires_in":"7200"}"#));
    assert_eq!(
        numeric.acquire(&[SCOPE]).unwrap().expires_on,
        Some(NOW + 7200)
    );

    let zero = Fixture::new(&[]);
    zero.imds
        .push(Reply::ok(r#"{"access_token":"tok","expires_in":0}"#));
    assert_eq!(zero.acquire(&[SCOPE]).unwrap().expires_on, Some(NOW));
}

fn token_expiry_failures() {
    let missing = Fixture::new(&[]);
    missing.imds.push(Reply::ok(r#"{"access_token":"tok"}"#));
    let error = missing.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(error.to_string(), format!("{NO_RESPONSE_PREFIX} "));

    let malformed = Fixture::new(&[]);
    malformed
        .imds
        .push(Reply::ok(r#"{"access_token":"tok","expires_in":"soon"}"#));
    assert_eq!(
        malformed.acquire(&[SCOPE]).unwrap_err().to_string(),
        format!("{NO_RESPONSE_PREFIX} ")
    );

    let payload = Fixture::new(&[]);
    payload.imds.push(Reply::ok(
        r#"{"error":"invalid_request","error_description":"no identity"}"#,
    ));
    assert_eq!(
        payload.acquire(&[SCOPE]).unwrap_err().to_string(),
        format!("{NO_RESPONSE_PREFIX} Token request error: (invalid_request) no identity")
    );

    let bare = Fixture::new(&[]);
    bare.imds.push(Reply::ok(r#"{"error":"bad"}"#));
    assert_eq!(
        bare.acquire(&[SCOPE]).unwrap_err().to_string(),
        format!("{NO_RESPONSE_PREFIX} Token request error: (bad) ")
    );
}

fn workload_environment() -> [(&'static str, &'static str); 5] {
    [
        ("AZURE_AUTHORITY_HOST", "https://login.contoso.test"),
        ("AZURE_TENANT_ID", "fixture-tenant"),
        ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://pod.test"),
    ]
}

fn workload_reply(fixture: &Fixture) {
    fixture.oidc.push(
        200,
        r#"{"access_token":"aad-token","expires_in":3600,"token_type":"Bearer"}"#,
    );
}

fn workload_selection() {
    workload_grant();
    workload_selection_details();
    workload_selection_rejections();
    workload_file_is_read_each_time();
}

fn workload_grant() {
    let fixture = Fixture::new(&workload_environment()).with_file("/var/run/token", "fixture-jwt");
    workload_reply(&fixture);
    let token = fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.token.expose_secret(), "aad-token");
    assert_eq!(token.expires_on, None);
    let requests = fixture.oidc.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].get("url").map(String::as_str),
        Some("https://login.contoso.test/fixture-tenant/oauth2/v2.0/token")
    );
    assert_eq!(
        requests[0].get("client_id").map(String::as_str),
        Some("env-client")
    );
    assert_eq!(
        requests[0].get("client_assertion").map(String::as_str),
        Some("fixture-jwt")
    );
    assert_eq!(requests[0].get("scope").map(String::as_str), Some(SCOPE));
    assert_eq!(
        requests[0].get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    assert_eq!(
        requests[0].get("client_assertion_type").map(String::as_str),
        Some("urn:ietf:params:oauth:client-assertion-type:jwt-bearer")
    );
    assert!(fixture.imds.recorded().is_empty());
}

fn workload_selection_details() {
    let argument = Fixture::new(&workload_environment()).with_file("/var/run/token", "fixture-jwt");
    workload_reply(&argument);
    argument
        .credential(Some("arg-client"))
        .token(&argument.io(), &[SCOPE])
        .unwrap();
    assert_eq!(
        argument.oidc.recorded()[0]
            .get("client_id")
            .map(String::as_str),
        Some("arg-client")
    );

    let incomplete = Fixture::new(&[
        ("AZURE_AUTHORITY_HOST", "https://login.contoso.test"),
        ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ("AZURE_CLIENT_ID", "env-client"),
    ]);
    incomplete.imds.push(Reply::ok(TOKEN));
    let selection = incomplete.credential(None).select().unwrap();
    assert_eq!(selection.kind, ManagedIdentityKind::Imds);

    let excluded = Fixture::new(&workload_environment()).with_file("/var/run/token", "fixture-jwt");
    let credential = super::super::ManagedIdentityCredential {
        environment: &excluded.environment,
        host: &excluded.host,
        client_id: None,
        exclude_workload_identity: true,
        within_default_credential: false,
        probe: ProbePolicy::default(),
    };
    assert_eq!(credential.select().unwrap().kind, ManagedIdentityKind::Imds);
}

fn workload_selection_rejections() {
    let missing = Fixture::new(&[
        ("AZURE_AUTHORITY_HOST", "https://login.contoso.test"),
        ("AZURE_TENANT_ID", "fixture-tenant"),
        ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
    ]);
    let error = missing.credential(None).select().unwrap_err();
    assert_eq!(
        error.to_string(),
        "Workload identity was selected but no client ID was provided. Configure the environment with a client ID or pass a value for \"client_id\" argument"
    );

    let missing_file = Fixture::new(&workload_environment());
    let file_error = missing_file.acquire(&[SCOPE]).unwrap_err();
    assert!(matches!(
        file_error,
        super::super::error::ManagedIdentityError::TokenFile { .. }
    ));
}

fn workload_file_is_read_each_time() {
    let fixture = Fixture::new(&workload_environment()).with_file("/var/run/token", "jwt-one");
    workload_reply(&fixture);
    workload_reply(&fixture);
    fixture.acquire(&[SCOPE]).unwrap();
    fixture.files.insert("/var/run/token", "jwt-two");
    fixture.acquire(&[SCOPE]).unwrap();
    let requests = fixture.oidc.recorded();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].get("client_assertion").map(String::as_str),
        Some("jwt-one")
    );
    assert_eq!(
        requests[1].get("client_assertion").map(String::as_str),
        Some("jwt-two")
    );
}

fn imds_retry_policy() {
    exhausted_ladder(410, &[5, 10, 20, 40].map(Duration::from_secs));
    exhausted_ladder(404, &[1600, 3200, 6400, 12800].map(Duration::from_millis));
    retry_mixed();
    retry_after();
    retry_400_not_retried();
    retry_transport_error();
}

fn exhausted_ladder(status: u16, expected: &[Duration]) {
    let fixture = Fixture::new(&[]);
    fixture
        .imds
        .push_all(repeat_n(Reply::json(status, "failing"), 6).collect());
    fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(fixture.imds.recorded().len(), 6);
    assert_eq!(fixture.scheduler.recorded(), expected);
}

fn retry_mixed() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::json(410, "gone"));
    fixture.imds.push(Reply::json(410, "gone"));
    fixture.imds.push(Reply::json(500, "boom"));
    fixture.imds.push(Reply::ok(TOKEN));
    let token = fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.token.expose_secret(), "imds-token");
    assert_eq!(
        fixture.scheduler.recorded(),
        vec![Duration::from_secs(5), Duration::from_millis(3200)]
    );
}

fn retry_after() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::Response {
        status: 429,
        headers: vec![("Retry-After", "7")],
        body: "slow".to_owned(),
    });
    fixture.imds.push(Reply::ok(TOKEN));
    let token = fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.token.expose_secret(), "imds-token");
    assert_eq!(fixture.scheduler.recorded(), vec![Duration::from_secs(7)]);
}

fn retry_400_not_retried() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push(Reply::bare(400, "boom"));
    fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(fixture.imds.recorded().len(), 1);
}

fn retry_transport_error() {
    let fixture = Fixture::new(&[]);
    fixture.imds.push_all(repeat_n(Reply::Failure, 4).collect());
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(error.to_string(), format!("{NO_RESPONSE_PREFIX} "));
    assert_eq!(fixture.imds.recorded().len(), 4);
    assert_eq!(
        fixture.scheduler.recorded(),
        vec![Duration::from_millis(1600), Duration::from_millis(3200)]
    );
}

fn hosted_selection_order() {
    hosted_variants();
    workload_and_unconfigured();
    presence_over_truthiness();
    arc_file_probe();
    arc_token();
}

fn selection_of(fixture: &Fixture) -> (ManagedIdentityKind, Option<&'static str>) {
    let credential = fixture.credential(None);
    let selection = credential.select().unwrap();
    let inner = credential
        .inner_protocol(&selection)
        .map(HostedProtocol::name);
    (selection.kind, inner)
}

fn hosted_variants() {
    let service_fabric = Fixture::new(&[
        ("IDENTITY_ENDPOINT", "http://sf.test/token"),
        ("IDENTITY_HEADER", "header-secret"),
        ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
    ]);
    assert_eq!(
        selection_of(&service_fabric),
        (ManagedIdentityKind::ServiceFabric, Some("service_fabric"))
    );
    let app_service = Fixture::new(&[
        ("IDENTITY_ENDPOINT", APP_SERVICE),
        ("IDENTITY_HEADER", "header-secret"),
    ]);
    assert_eq!(
        selection_of(&app_service),
        (ManagedIdentityKind::AppService, Some("app_service"))
    );
    let arc = Fixture::new(&[
        ("IDENTITY_ENDPOINT", "http://arc.test/token"),
        ("IMDS_ENDPOINT", "http://arc.test/imds"),
    ]);
    assert_eq!(selection_of(&arc), (ManagedIdentityKind::Arc, Some("arc")));
    let azure_ml = Fixture::new(&[
        ("MSI_ENDPOINT", "http://ml.test/token"),
        ("MSI_SECRET", "ml-secret"),
    ]);
    assert_eq!(
        selection_of(&azure_ml),
        (ManagedIdentityKind::AzureMl, Some("azure_ml"))
    );
}

fn workload_and_unconfigured() {
    let cloud_shell = Fixture::new(&[("MSI_ENDPOINT", "http://ml.test/token")]);
    assert_eq!(
        selection_of(&cloud_shell),
        (ManagedIdentityKind::CloudShell, None)
    );
    let workload = Fixture::new(&[
        ("AZURE_AUTHORITY_HOST", "https://login.contoso.test"),
        ("AZURE_TENANT_ID", "fixture-tenant"),
        ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ("AZURE_CLIENT_ID", "env-client"),
    ]);
    assert_eq!(
        selection_of(&workload),
        (ManagedIdentityKind::Workload, None)
    );
    let imds = Fixture::new(&[]);
    assert_eq!(selection_of(&imds), (ManagedIdentityKind::Imds, Some("vm")));
    assert!(imds.imds.recorded().is_empty());
    let lone = Fixture::new(&[("IDENTITY_ENDPOINT", APP_SERVICE)]);
    assert_eq!(
        selection_of(&lone),
        (ManagedIdentityKind::Unconfigured, None)
    );
}

fn presence_over_truthiness() {
    let empty_all = Fixture::new(&[
        ("IDENTITY_ENDPOINT", ""),
        ("IDENTITY_HEADER", ""),
        ("IDENTITY_SERVER_THUMBPRINT", ""),
    ]);
    assert_eq!(
        selection_of(&empty_all),
        (ManagedIdentityKind::Imds, Some("service_fabric"))
    );
    let empty_pair = Fixture::new(&[("IDENTITY_ENDPOINT", ""), ("IDENTITY_HEADER", "")]);
    assert_eq!(
        selection_of(&empty_pair),
        (ManagedIdentityKind::Imds, Some("app_service"))
    );
    let empty_ml = Fixture::new(&[("MSI_ENDPOINT", ""), ("MSI_SECRET", "")]);
    assert_eq!(
        selection_of(&empty_ml),
        (ManagedIdentityKind::Imds, Some("azure_ml"))
    );
    let empty_arc = Fixture::new(&[
        ("IDENTITY_ENDPOINT", ""),
        ("IMDS_ENDPOINT", "http://arc.test/imds"),
    ]);
    assert_eq!(
        selection_of(&empty_arc),
        (ManagedIdentityKind::Imds, Some("vm"))
    );
}

fn arc_file_probe() {
    let fixture = Fixture::new(&[]).with_file("/opt/azcmagent/bin/himds", "");
    assert_eq!(
        selection_of(&fixture),
        (ManagedIdentityKind::Imds, Some("arc"))
    );
}

fn arc_token() {
    let fixture = Fixture::new(&[
        ("IDENTITY_ENDPOINT", "http://arc.test/token"),
        ("IMDS_ENDPOINT", "http://arc.test/imds"),
    ])
    .with_file("/var/opt/azcmagent/tokens/secret.key", "arc-secret");
    fixture.arc.push(Reply::Response {
        status: 401,
        headers: vec![("WWW-Authenticate", "Basic realm=secret.key")],
        body: String::new(),
    });
    fixture.arc.push(Reply::ok(
        r#"{"access_token":"arc-token","expires_in":3600}"#,
    ));
    let token = fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.token.expose_secret(), "arc-token");
    assert_eq!(token.expires_on, Some(NOW + 3600));
    assert_eq!(token.refresh_on, None);
    assert!(fixture.imds.recorded().is_empty());
}

fn partial_identity_endpoint() {
    let fixture = Fixture::new(&[("IDENTITY_ENDPOINT", "http://sf.test/token")]);
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(error.to_string(), NO_ENDPOINT);
    assert!(fixture.imds.recorded().is_empty());
    assert!(fixture.hosted.recorded().is_empty());
}

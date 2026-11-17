use std::env::temp_dir;
use std::fs;
use std::iter::repeat_n;
use std::process::id;
use std::time::Duration;

use betterlite_redact::secret::Redactor;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::super::error::{ManagedIdentityError, NO_ENDPOINT, NO_RESPONSE_PREFIX};
use super::super::imds::{CLIENT_SKU, CLIENT_VERSION, ProbePolicy};
use super::super::runtime::{FsTokenFiles, TokenFiles as _};
use super::super::selection::ManagedIdentityKind;
use super::fixture::{Fixture, NOW, REQUEST_ID, Reply, redactor};
use super::selection::hosted_selection_order;

pub(super) const SCOPE: &str = "https://cognitiveservices.azure.com/.default";
const RESOURCE: &str = "https://cognitiveservices.azure.com";
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
fn azure_managed_identity_cases(#[case] case: fn(&Redactor)) {
    case(&redactor());
}

fn system_assigned_imds(redactor: &Redactor) {
    let fixture = Fixture::new(
        &[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://pod.test/")],
        redactor,
    );
    fixture.imds.push(Reply::ok(TOKEN));
    let token = fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.token.expose_secret(), "imds-token");
    assert_eq!(token.expires_on, Some(NOW + 3600));
    assert_eq!(token.token_type.as_deref(), Some("Bearer"));
    assert_eq!(token.refresh_on, None);
    let requests = fixture.imds.recorded();
    assert_eq!(requests.len(), 1);
    imds_request_shape(&requests[0]);
    default_authority(redactor);
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
    assert_eq!(request.timeout_seconds, 2);
}

fn default_authority(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture.imds.push(Reply::ok(TOKEN));
    fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(
        fixture.imds.recorded()[0].url,
        "http://169.254.169.254/metadata/identity/oauth2/token"
    );
}

fn client_id_query(redactor: &Redactor) {
    let user_assigned = Fixture::new(
        &[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://pod.test")],
        redactor,
    );
    user_assigned.imds.push(Reply::ok(TOKEN));
    user_assigned
        .credential(Some("fixture-client"))
        .token(&user_assigned.io(), &[SCOPE])
        .unwrap();
    let requests = user_assigned.imds.recorded();
    assert_eq!(requests[0].query("client_id"), Some("fixture-client"));
    assert_eq!(requests[0].query("object_id"), None);
    assert_eq!(requests[0].query("resource_id"), None);

    let system_assigned = Fixture::new(
        &[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://pod.test")],
        redactor,
    );
    system_assigned.imds.push(Reply::ok(TOKEN));
    system_assigned
        .credential(Some(""))
        .token(&system_assigned.io(), &[SCOPE])
        .unwrap();
    assert_eq!(system_assigned.imds.recorded()[0].query("client_id"), None);
}

fn imds_unavailable(redactor: &Redactor) {
    chained_probes(redactor);
    unavailable_replies(redactor);
    status_500(redactor);
    status_403(redactor);
}

fn chained_probes(redactor: &Redactor) {
    probe_in_chain(redactor);
    probe_no_response(redactor);
    probe_forbidden_unreachable(redactor);
    probe_forbidden_other(redactor);
    pod_authority_skips_probe(redactor);
}

fn chained_probe(
    fixture: &Fixture<'_>,
) -> Result<super::super::ManagedIdentityToken, ManagedIdentityError> {
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

fn probe_in_chain(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture.imds.push(Reply::ok(TOKEN));
    fixture.imds.push(Reply::ok(TOKEN));
    chained_probe(&fixture).unwrap();
    let requests = fixture.imds.recorded();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].headers.len(), 1);
    assert_eq!(
        requests[0].header("x-ms-client-request-id"),
        Some(REQUEST_ID)
    );
    assert_eq!(requests[0].header("metadata"), None);
    assert_eq!(requests[0].timeout_seconds, 1);
    assert_eq!(requests[1].header("metadata"), Some("true"));
    assert_eq!(requests[1].timeout_seconds, 2);
    assert!(fixture.scheduler.recorded().is_empty());
}

fn probe_no_response(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture.imds.push(Reply::Failure);
    let error = chained_probe(&fixture).unwrap_err();
    assert_eq!(fixture.imds.recorded().len(), 1);
    assert_eq!(error.to_string(), format!("{NO_RESPONSE_PREFIX} "));
}

fn probe_forbidden_unreachable(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
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

fn probe_forbidden_other(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture.imds.push(Reply::json(403, r#"{"error":"denied"}"#));
    fixture.imds.push(Reply::ok(TOKEN));
    chained_probe(&fixture).unwrap();
    assert_eq!(fixture.imds.recorded().len(), 2);
}

fn unavailable_replies(redactor: &Redactor) {
    for (reply, message) in [
        (
            Reply::bare(400, "boom"),
            "ManagedIdentityCredential authentication unavailable. No identity has been assigned to this resource.",
        ),
        (
            Reply::text(200, "not-json"),
            "ManagedIdentityCredential authentication unavailable.",
        ),
    ] {
        let fixture = Fixture::new(&[], redactor);
        fixture.imds.push(reply);
        let error = fixture.acquire(&[SCOPE]).unwrap_err();
        assert_eq!(error.to_string(), message);
        assert_eq!(fixture.imds.recorded().len(), 1, "{message}");
    }
}

fn status_500(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture
        .imds
        .push_all(repeat_n(Reply::json(500, "boom"), 6).collect());
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    assert!(matches!(
        error,
        ManagedIdentityError::ClientAuthentication { .. }
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

fn status_403(redactor: &Redactor) {
    let unreachable = Fixture::new(&[], redactor);
    unreachable.imds.push(Reply::bare(403, "unreachable host"));
    assert_eq!(
        unreachable.acquire(&[SCOPE]).unwrap_err().to_string(),
        "ManagedIdentityCredential authentication unavailable. Error: Unexpected response \"unreachable host\""
    );
    let denied = Fixture::new(&[], redactor);
    denied.imds.push(Reply::bare(403, "denied"));
    assert!(matches!(
        denied.acquire(&[SCOPE]).unwrap_err(),
        ManagedIdentityError::ClientAuthentication { .. }
    ));
    let retried = Fixture::new(&[], redactor);
    retried
        .imds
        .push_all(repeat_n(Reply::bare(500, "unreachable"), 6).collect());
    assert!(matches!(
        retried.acquire(&[SCOPE]).unwrap_err(),
        ManagedIdentityError::ClientAuthentication { .. }
    ));
}

fn pod_authority_skips_probe(redactor: &Redactor) {
    let fixture = Fixture::new(&[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "")], redactor);
    fixture.imds.push(Reply::ok(TOKEN));
    chained_probe(&fixture).unwrap();
    let requests = fixture.imds.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].header("metadata"), Some("true"));
    assert_eq!(requests[0].url, "/metadata/identity/oauth2/token");
}

fn token_expiry(redactor: &Redactor) {
    token_expiry_success(redactor);
    token_expiry_failures(redactor);
}

fn token_expiry_success(redactor: &Redactor) {
    let full = Fixture::new(&[], redactor);
    full.imds.push(Reply::ok(
        r#"{"access_token":"tok","expires_in":7200,"token_type":"Custom"}"#,
    ));
    let token = full.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.expires_on, Some(NOW + 7200));
    assert_eq!(token.token_type.as_deref(), Some("Custom"));
    assert_eq!(token.refresh_on, Some(NOW + 3600));

    let numeric = Fixture::new(&[], redactor);
    numeric
        .imds
        .push(Reply::ok(r#"{"access_token":"tok","expires_in":"7200"}"#));
    assert_eq!(
        numeric.acquire(&[SCOPE]).unwrap().expires_on,
        Some(NOW + 7200)
    );

    let zero = Fixture::new(&[], redactor);
    zero.imds
        .push(Reply::ok(r#"{"access_token":"tok","expires_in":0}"#));
    assert_eq!(zero.acquire(&[SCOPE]).unwrap().expires_on, Some(NOW));
}

fn token_expiry_failures(redactor: &Redactor) {
    let missing = Fixture::new(&[], redactor);
    missing.imds.push(Reply::ok(r#"{"access_token":"tok"}"#));
    let error = missing.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(error.to_string(), format!("{NO_RESPONSE_PREFIX} "));

    let malformed = Fixture::new(&[], redactor);
    malformed
        .imds
        .push(Reply::ok(r#"{"access_token":"tok","expires_in":"soon"}"#));
    assert_eq!(
        malformed.acquire(&[SCOPE]).unwrap_err().to_string(),
        format!("{NO_RESPONSE_PREFIX} ")
    );

    let payload = Fixture::new(&[], redactor);
    payload.imds.push(Reply::ok(
        r#"{"error":"invalid_request","error_description":"no identity"}"#,
    ));
    assert_eq!(
        payload.acquire(&[SCOPE]).unwrap_err().to_string(),
        format!("{NO_RESPONSE_PREFIX} Token request error: (invalid_request) no identity")
    );

    let bare = Fixture::new(&[], redactor);
    bare.imds.push(Reply::ok(r#"{"error":"bad"}"#));
    assert_eq!(
        bare.acquire(&[SCOPE]).unwrap_err().to_string(),
        format!("{NO_RESPONSE_PREFIX} Token request error: (bad) ")
    );
}

pub(super) fn workload_environment() -> [(&'static str, &'static str); 5] {
    [
        ("AZURE_AUTHORITY_HOST", "https://login.contoso.test"),
        ("AZURE_TENANT_ID", "fixture-tenant"),
        ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ("AZURE_CLIENT_ID", "env-client"),
        ("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://pod.test"),
    ]
}

fn workload_reply(fixture: &Fixture<'_>) {
    fixture.oidc.push(
        200,
        r#"{"access_token":"aad-token","expires_in":3600,"token_type":"Bearer"}"#,
    );
}

fn workload_selection(redactor: &Redactor) {
    workload_grant(redactor);
    workload_selection_details(redactor);
    workload_selection_rejections(redactor);
    workload_file_is_read_each_time(redactor);
    filesystem_token_file();
}

fn filesystem_token_file() {
    let path = temp_dir().join(format!("betterlite-b676-{}", id()));
    fs::write(&path, b"fixture-jwt").unwrap();
    let read = FsTokenFiles.read(path.to_str().unwrap());
    fs::remove_file(&path).unwrap();
    assert_eq!(read.unwrap(), b"fixture-jwt");
    FsTokenFiles
        .read("/nonexistent/betterlite-b676")
        .unwrap_err();
}

fn workload_grant(redactor: &Redactor) {
    let fixture =
        Fixture::new(&workload_environment(), redactor).with_file("/var/run/token", "fixture-jwt");
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

fn workload_selection_details(redactor: &Redactor) {
    let argument =
        Fixture::new(&workload_environment(), redactor).with_file("/var/run/token", "fixture-jwt");
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

    let incomplete = Fixture::new(
        &[
            ("AZURE_AUTHORITY_HOST", "https://login.contoso.test"),
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
            ("AZURE_CLIENT_ID", "env-client"),
        ],
        redactor,
    );
    incomplete.imds.push(Reply::ok(TOKEN));
    let selection = incomplete.credential(None).select().unwrap();
    assert_eq!(selection.kind, ManagedIdentityKind::Imds);

    let excluded =
        Fixture::new(&workload_environment(), redactor).with_file("/var/run/token", "fixture-jwt");
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

fn workload_selection_rejections(redactor: &Redactor) {
    let missing = Fixture::new(
        &[
            ("AZURE_AUTHORITY_HOST", "https://login.contoso.test"),
            ("AZURE_TENANT_ID", "fixture-tenant"),
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
        ],
        redactor,
    );
    let error = missing.credential(None).select().unwrap_err();
    assert_eq!(
        error.to_string(),
        "Workload identity was selected but no client ID was provided. Configure the environment with a client ID or pass a value for \"client_id\" argument"
    );

    let missing_file = Fixture::new(&workload_environment(), redactor);
    let file_error = missing_file.acquire(&[SCOPE]).unwrap_err();
    assert!(matches!(file_error, ManagedIdentityError::TokenFile { .. }));
}

fn workload_file_is_read_each_time(redactor: &Redactor) {
    let fixture =
        Fixture::new(&workload_environment(), redactor).with_file("/var/run/token", "jwt-one");
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

fn imds_retry_policy(redactor: &Redactor) {
    exhausted_ladder(410, &[5, 10, 20, 40].map(Duration::from_secs), redactor);
    exhausted_ladder(
        404,
        &[1600, 3200, 6400, 12800].map(Duration::from_millis),
        redactor,
    );
    retry_mixed(redactor);
    retry_connection_then_status(redactor);
    retry_after_headers(redactor);
    retry_zero_header_backs_off(redactor);
    retry_status_codes(redactor);
    retry_transport_error(redactor);
}

fn exhausted_ladder(status: u16, expected: &[Duration], redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture
        .imds
        .push_all(repeat_n(Reply::json(status, "failing"), 6).collect());
    fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(fixture.imds.recorded().len(), 6);
    assert_eq!(fixture.scheduler.recorded(), expected);
}

fn retry_mixed(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
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

// connection errors and error statuses spend one total budget and share one backoff history
fn retry_connection_then_status(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture.imds.push_all(repeat_n(Reply::Failure, 3).collect());
    fixture
        .imds
        .push_all(repeat_n(Reply::json(500, "boom"), 3).collect());
    fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(fixture.imds.recorded().len(), 6);
    assert_eq!(
        fixture.scheduler.recorded(),
        [1600, 3200, 6400, 12800].map(Duration::from_millis)
    );
}

// a non-empty Retry-After header makes any error status retryable; the millisecond headers only set the wait
fn retry_after_headers(redactor: &Redactor) {
    let future = "Tue, 14 Nov 2023 22:13:50 GMT";
    let past = "Tue, 14 Nov 2023 22:13:10 GMT";
    let cases = [
        (429, "Retry-After", "7", 2, 7000),
        (400, "Retry-After", "7", 2, 7000),
        (400, "Retry-After", future, 2, 30_000),
        (400, "Retry-After", past, 2, 0),
        (400, "Retry-After", "0", 2, 0),
        (500, "Retry-After", "soon", 2, 0),
        (400, "Retry-After", "", 1, 0),
        (200, "Retry-After", "7", 1, 0),
        (503, "x-ms-retry-after-ms", "1500", 2, 1500),
        (503, "retry-after-ms", "250", 2, 250),
        (400, "x-ms-retry-after-ms", "1500", 1, 0),
    ];
    for (status, name, value, requests, sleep_millis) in cases {
        let fixture = Fixture::new(&[], redactor);
        fixture.imds.push(Reply::Response {
            status,
            headers: vec![(name, value)],
            body: TOKEN.to_owned(),
        });
        fixture.imds.push(Reply::ok(TOKEN));
        let _outcome = fixture.acquire(&[SCOPE]);
        let expected = if sleep_millis == 0 {
            Vec::new()
        } else {
            vec![Duration::from_millis(sleep_millis)]
        };
        assert_eq!(
            fixture.imds.recorded().len(),
            requests,
            "{status} {name} {value}"
        );
        assert_eq!(
            fixture.scheduler.recorded(),
            expected,
            "{status} {name} {value}"
        );
    }
}

// a zero Retry-After is no wait at all, so the backoff of the second retry applies
fn retry_zero_header_backs_off(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture.imds.push(Reply::bare(404, "missing"));
    fixture.imds.push(Reply::Response {
        status: 429,
        headers: vec![("Retry-After", "0")],
        body: "slow".to_owned(),
    });
    fixture.imds.push(Reply::ok(TOKEN));
    fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(fixture.scheduler.recorded(), [Duration::from_millis(1600)]);
}

fn retry_status_codes(redactor: &Redactor) {
    let cases = [(400, 1), (403, 1), (408, 2), (505, 2), (998, 2), (999, 1)];
    for (status, requests) in cases {
        let fixture = Fixture::new(&[], redactor);
        fixture.imds.push(Reply::bare(status, "boom"));
        fixture.imds.push(Reply::ok(TOKEN));
        let _outcome = fixture.acquire(&[SCOPE]);
        assert_eq!(fixture.imds.recorded().len(), requests, "{status}");
    }
}

fn retry_transport_error(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor);
    fixture.imds.push_all(repeat_n(Reply::Failure, 4).collect());
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(error.to_string(), format!("{NO_RESPONSE_PREFIX} "));
    assert_eq!(fixture.imds.recorded().len(), 4);
    assert_eq!(
        fixture.scheduler.recorded(),
        vec![Duration::from_millis(1600), Duration::from_millis(3200)]
    );
}

fn partial_identity_endpoint(redactor: &Redactor) {
    let fixture = Fixture::new(&[("IDENTITY_ENDPOINT", "http://sf.test/token")], redactor);
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    assert_eq!(error.to_string(), NO_ENDPOINT);
    assert!(fixture.imds.recorded().is_empty());
    assert!(fixture.hosted.recorded().is_empty());
}

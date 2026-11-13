use secrecy::ExposeSecret as _;

use super::super::selection::ManagedIdentityKind;
use super::fixture::{APP_SERVICE, Fixture, RecordedHosted, Reply};

const SCOPE: &str = "https://cognitiveservices.azure.com/.default";
const SECRET: &str = "fixture-identity-header-secret";
const THUMBPRINT: &str = "00112233445566778899aabbccddeeff00112233";

fn header(request: &RecordedHosted, name: &str) -> Option<String> {
    request
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.expose_secret().to_owned())
}

#[test]
fn azure_managed_identity_credential_identity_isolation() {
    isolation_between_headers();
    isolation_between_client_ids();
}

fn isolation_between_headers() {
    let first = Fixture::new(&[
        ("IDENTITY_ENDPOINT", "http://app.test/first"),
        ("IDENTITY_HEADER", "first-header-secret"),
    ]);
    let second = Fixture::new(&[
        ("IDENTITY_ENDPOINT", "http://app.test/second"),
        ("IDENTITY_HEADER", "second-header-secret"),
    ]);
    first.hosted.push(Reply::ok(
        r#"{"access_token":"first-token","expires_on":1700003600}"#,
    ));
    second.hosted.push(Reply::ok(
        r#"{"access_token":"second-token","expires_on":1700003600}"#,
    ));
    let first_token = first.credential(None).token(&first.io(), &[SCOPE]).unwrap();
    let second_token = second
        .credential(None)
        .token(&second.io(), &[SCOPE])
        .unwrap();
    assert_eq!(first_token.token.expose_secret(), "first-token");
    assert_eq!(second_token.token.expose_secret(), "second-token");
    let first_requests = first.hosted.recorded();
    let second_requests = second.hosted.recorded();
    assert_eq!(first_requests.len(), 1);
    assert_eq!(second_requests.len(), 1);
    assert_eq!(first_requests[0].url, "http://app.test/first");
    assert_eq!(second_requests[0].url, "http://app.test/second");
    assert_eq!(
        header(&first_requests[0], "x-identity-header").as_deref(),
        Some("first-header-secret")
    );
    assert_eq!(
        header(&second_requests[0], "x-identity-header").as_deref(),
        Some("second-header-secret")
    );
    assert!(!format!("{:?}", first_requests[0]).contains("second-header-secret"));
    assert!(!format!("{:?}", second_requests[0]).contains("first-header-secret"));
}

fn isolation_between_client_ids() {
    let first = Fixture::new(&[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://first.test")]);
    let second = Fixture::new(&[("AZURE_POD_IDENTITY_AUTHORITY_HOST", "http://second.test")]);
    first.imds.push(Reply::ok(
        r#"{"access_token":"first-token","expires_in":3600}"#,
    ));
    second.imds.push(Reply::ok(
        r#"{"access_token":"second-token","expires_in":3600}"#,
    ));
    first
        .credential(Some("first-client"))
        .token(&first.io(), &[SCOPE])
        .unwrap();
    second
        .credential(Some("second-client"))
        .token(&second.io(), &[SCOPE])
        .unwrap();
    let first_requests = first.imds.recorded();
    let second_requests = second.imds.recorded();
    assert_eq!(
        first_requests[0].url,
        "http://first.test/metadata/identity/oauth2/token"
    );
    assert_eq!(
        second_requests[0].url,
        "http://second.test/metadata/identity/oauth2/token"
    );
    assert_eq!(first_requests[0].query("client_id"), Some("first-client"));
    assert_eq!(second_requests[0].query("client_id"), Some("second-client"));
}

#[test]
fn azure_managed_identity_secret_safe_failure() {
    let fixture = Fixture::new(&[
        ("IDENTITY_ENDPOINT", APP_SERVICE),
        ("IDENTITY_HEADER", SECRET),
        ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
    ]);
    fixture.hosted.push(Reply::ok(&format!(
        r#"{{"access_token":"fixture-access-token","error":{{"code":"SecretHeaderNotFound"}},"echo":"{SECRET}"}}"#
    )));
    let error = fixture.acquire(&[SCOPE]).unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(shown.contains("[redacted]"));
    assert!(!shown.contains(SECRET));
    assert!(!shown.contains("fixture-access-token"));
    assert!(!format!("{:?}", fixture.environment).contains(SECRET));
    assert!(
        fixture
            .credential(None)
            .select()
            .is_ok_and(|selection| selection.kind == ManagedIdentityKind::ServiceFabric)
    );
}

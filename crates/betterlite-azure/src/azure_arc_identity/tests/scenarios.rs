#![cfg(test)]

use std::io::ErrorKind;

use secrecy::ExposeSecret as _;

use super::fixture::{ENDPOINT, FixtureHost, FixtureTransport, RESOURCE, Recorded, response, run};
use crate::azure_arc_identity::error::ArcIdentityError;
use crate::azure_arc_identity::{ArcIdentity, ArcPlatform, acquire_arc_token, select_arc_endpoint};

const PREFIX: &str = "Azure Arc managed identity configuration not found in environment. ";
const TOKEN_BODY: &str = r#"{"access_token":"tok","expires_in":"3600"}"#;
const KEY_PATH: &str = "/var/opt/azcmagent/tokens/abc.key";

pub(super) fn endpoint_selection() {
    let mut hosted = FixtureHost::linux(&[]);
    hosted
        .environment
        .insert("IDENTITY_ENDPOINT".to_owned(), "http://arc".to_owned());
    assert_eq!(select_arc_endpoint(&hosted), None);
    hosted
        .environment
        .insert("IMDS_ENDPOINT".to_owned(), "http://imds".to_owned());
    assert_eq!(select_arc_endpoint(&hosted), Some("http://arc".to_owned()));
    hosted
        .environment
        .insert("IDENTITY_ENDPOINT".to_owned(), String::new());
    assert_eq!(select_arc_endpoint(&hosted), None);

    let mut linux = FixtureHost::linux(&[("/opt/azcmagent/bin/himds", "")]);
    assert_eq!(select_arc_endpoint(&linux), Some(ENDPOINT.to_owned()));
    linux.files.clear();
    assert_eq!(select_arc_endpoint(&linux), None);

    let himds = "C:\\Program Files\\AzureConnectedMachineAgent\\himds.exe";
    let mut windows = FixtureHost::linux(&[(himds, "")]);
    windows.platform = ArcPlatform::Windows;
    assert_eq!(select_arc_endpoint(&windows), None);
    windows.program_files = "C:\\Program Files".to_owned();
    assert_eq!(select_arc_endpoint(&windows), Some(ENDPOINT.to_owned()));

    let literal = "${ProgramFiles}\\AzureConnectedMachineAgent\\himds.exe";
    let mut unset_program_files = FixtureHost::linux(&[(literal, "")]);
    unset_program_files.platform = ArcPlatform::Windows;
    assert_eq!(
        select_arc_endpoint(&unset_program_files),
        Some(ENDPOINT.to_owned())
    );
    windows.platform = ArcPlatform::Other("darwin".to_owned());
    assert_eq!(select_arc_endpoint(&windows), None);
}

pub(super) fn challenge_request() {
    let host = FixtureHost::linux(&[(KEY_PATH, "key-material")]);
    let transport = FixtureTransport::new(vec![
        response(200, &[("www-authenticate", "BASIC REALM=/x/abc.key")], ""),
        response(200, &[], TOKEN_BODY),
    ]);
    let token = acquire_arc_token(
        &host,
        &transport,
        ENDPOINT,
        RESOURCE,
        ArcIdentity::SystemAssigned,
    )
    .unwrap();
    assert_eq!(token.token.expose_secret(), "tok");
    let first = transport.recorded().remove(0);
    assert_eq!(first, Recorded::challenge());

    for header in [
        "Basic realm=a=b",
        "Digest realm=x",
        " Basic realm=abc",
        "Basic",
    ] {
        let (outcome, sent) = run(&host, header, TOKEN_BODY);
        let message = outcome.unwrap_err().to_string();
        assert!(message.starts_with(&format!("{PREFIX}Unrecognizable WWW-Authenticate header")));
        assert_eq!(sent.recorded().len(), 1, "{header}");
    }
    missing_header(&host);
}

fn missing_header(host: &FixtureHost) {
    let transport = FixtureTransport::new(vec![response(401, &[], "")]);
    let outcome = acquire_arc_token(
        host,
        &transport,
        ENDPOINT,
        RESOURCE,
        ArcIdentity::SystemAssigned,
    );
    assert!(matches!(
        outcome,
        Err(ArcIdentityError::UnrecognizableChallenge { .. })
    ));
}

pub(super) fn restricted_key() {
    let linux = FixtureHost::linux(&[
        ("/var/opt/azcmagent/tokens/passwd.key", "one"),
        ("/var/opt/azcmagent/tokens/x.y.key", "two"),
        ("/var/opt/azcmagent/tokens/abc.key", "three"),
        ("/var/opt/azcmagent/tokens/.hidden.key", "four"),
    ]);
    for (challenge, secret) in [
        ("/etc/passwd.txt", "one"),
        ("../../x.y.z", "two"),
        ("abc", "three"),
        ("/a/b/.hidden", "four"),
    ] {
        let (outcome, sent) = run(&linux, &format!("Basic realm={challenge}"), TOKEN_BODY);
        outcome.unwrap();
        assert_eq!(
            sent.recorded()[1].authorization(),
            Some(format!("Basic {secret}"))
        );
    }
    let mut windows =
        FixtureHost::linux(&[("D:\\PD\\AzureConnectedMachineAgent\\Tokens\\b.key", "win")]);
    windows.platform = ArcPlatform::Windows;
    windows.program_data = "D:\\PD".to_owned();
    for challenge in ["C:\\other\\dir\\b.txt", "C:b.txt", "b.txt"] {
        let (outcome, sent) = run(&windows, &format!("Basic realm={challenge}"), TOKEN_BODY);
        outcome.unwrap();
        assert_eq!(
            sent.recorded()[1].authorization(),
            Some("Basic win".to_owned())
        );
    }
    let drive_like = FixtureHost::linux(&[("/var/opt/azcmagent/tokens/C:b.key", "drive")]);
    let (drive_outcome, drive_sent) = run(&drive_like, "Basic realm=C:b.txt", TOKEN_BODY);
    drive_outcome.unwrap();
    assert_eq!(
        drive_sent.recorded()[1].authorization(),
        Some("Basic drive".to_owned())
    );
    key_file_limits();
}

fn key_file_limits() {
    let at_limit = "a".repeat(4096);
    let limit_host = FixtureHost::linux(&[(KEY_PATH, at_limit.as_str())]);
    run(&limit_host, "Basic realm=abc", TOKEN_BODY).0.unwrap();
    let over_limit = "a".repeat(4097);
    let over_host = FixtureHost::linux(&[(KEY_PATH, over_limit.as_str())]);
    let (too_big, sent) = run(&over_host, "Basic realm=abc", TOKEN_BODY);
    assert_eq!(
        too_big.unwrap_err().to_string(),
        format!("{PREFIX}Local key file shall not be larger than 4KB")
    );
    assert_eq!(sent.recorded().len(), 1);

    let (missing, _) = run(&FixtureHost::linux(&[]), "Basic realm=abc", TOKEN_BODY);
    assert!(matches!(
        missing,
        Err(ArcIdentityError::KeyFile(source)) if source.kind() == ErrorKind::NotFound
    ));
    let mut darwin = FixtureHost::linux(&[]);
    darwin.platform = ArcPlatform::Other("darwin".to_owned());
    let (unsupported, _) = run(&darwin, "Basic realm=abc", TOKEN_BODY);
    assert_eq!(
        unsupported.unwrap_err().to_string(),
        format!("{PREFIX}Platform darwin was undefined and unsupported")
    );
}

pub(super) fn second_request() {
    let host = FixtureHost::linux(&[(KEY_PATH, "s3cret\r\nline\r\n")]);
    let (outcome, sent) = run(&host, "Basic realm=/q/abc.key", TOKEN_BODY);
    outcome.unwrap();
    assert_eq!(
        sent.recorded(),
        vec![
            Recorded::challenge(),
            Recorded::authenticated("s3cret\nline\n"),
        ]
    );
}

pub(super) fn token_reply() {
    let host = FixtureHost::linux(&[(KEY_PATH, "key-material")]);
    let accepted = run(
        &host,
        "Basic realm=abc",
        r#"{"access_token":"token-value","expires_in":3600}"#,
    )
    .0
    .unwrap();
    assert_eq!(accepted.expires_on, 1_700_003_600);
    assert_eq!(accepted.token_type, "Bearer");
    assert_eq!(accepted.refresh_on, None);
    let long_body = r#"{"access_token":"token-value","expires_in":"7200","token_type":"Custom","resource":"r"}"#;
    let long_lived = run(&host, "Basic realm=abc", long_body).0.unwrap();
    assert_eq!(long_lived.expires_on, 1_700_007_200);
    assert_eq!(long_lived.token_type, "Custom");
    assert_eq!(long_lived.refresh_on, Some(1_700_003_600));
    let negative_body = r#"{"access_token":"token-value","expires_in":-5}"#;
    let negative = run(&host, "Basic realm=abc", negative_body).0.unwrap();
    assert_eq!(negative.expires_on, 1_699_999_995);
    rejected_replies(&host);
}

fn rejected_replies(host: &FixtureHost) {
    for (body, shown) in [
        ("not json", "not json"),
        (r#"{"expires_in":10}"#, r#"{"expires_in":10}"#),
        (
            r#"{"access_token":"","expires_in":10}"#,
            r#"{"access_token":"","expires_in":10}"#,
        ),
        (
            r#"{"access_token":"abc","expires_in":10"#,
            r#"{"access_token":"abc","expires_in":10"#,
        ),
        (
            r#"{"access_token":"token-value"}"#,
            r#"{"access_token":"[redacted]"}"#,
        ),
        (
            r#"{"access_token":"token-value","expires_in":0}"#,
            r#"{"access_token":"[redacted]","expires_in":0}"#,
        ),
        (
            r#"{"access_token":"token-value","expires_in":""}"#,
            r#"{"access_token":"[redacted]","expires_in":""}"#,
        ),
    ] {
        let rejected = run(host, "Basic realm=abc", body).0.unwrap_err();
        assert_eq!(
            rejected.to_string(),
            format!("{PREFIX}Token request error: (invalid_request) {shown}")
        );
    }
    let text_body = r#"{"access_token":"token-value","expires_in":"soon"}"#;
    let unusable = run(host, "Basic realm=abc", text_body).0.unwrap_err();
    assert!(matches!(unusable, ArcIdentityError::ReplyValue { path } if path == "expires_in"));
    let array = run(host, "Basic realm=abc", r#"["access_token"]"#)
        .0
        .unwrap_err();
    assert!(matches!(array, ArcIdentityError::ReplyValue { .. }));
}

pub(super) fn user_identity() {
    let host = FixtureHost::linux(&[(KEY_PATH, "key-material")]);
    let transport = FixtureTransport::challenge_then("Basic realm=abc", TOKEN_BODY);
    let rejected = acquire_arc_token(
        &host,
        &transport,
        ENDPOINT,
        RESOURCE,
        ArcIdentity::UserAssigned,
    )
    .unwrap_err();
    assert_eq!(
        rejected.to_string(),
        format!(
            "{PREFIX}Invalid managed_identity parameter. Azure Arc supports only system-assigned managed identity, See also https://learn.microsoft.com/en-us/azure/service-fabric/configure-existing-cluster-enable-managed-identity-token-service"
        )
    );
    assert!(transport.recorded().is_empty());
}

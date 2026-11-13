#![cfg(test)]

use secrecy::{ExposeSecret as _, SecretString};

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::credentials::role::RoleAuth;
use crate::credentials::role::aws_role::auth_with_aws_role;

use super::support::{Fixture, Reply, Transport, load, settings};

const TARGET: &str = "arn:aws:iam::123456789012:role/TargetRole";

const AMBIENT: &[(&str, &str)] = &[
    ("AWS_ACCESS_KEY_ID", "ambient"),
    ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
    ("AWS_EC2_METADATA_DISABLED", "true"),
];

const DENIED_IDENTITY: Reply = Reply::Error {
    status: 403,
    code: "AccessDenied",
    message: "the fixture identity call is denied",
};

fn role_reply(access: &'static str) -> Reply {
    Reply::Credentials {
        access,
        secret: "role-secret",
        token: "role-token",
        expiration: "2026-01-01T01:00:00Z",
    }
}

fn resolve_one(fixture: &Fixture, params: &str) -> Credentials {
    fixture
        .runtime
        .block_on(
            fixture
                .resolver()
                .get_credentials(&settings(params, AMBIENT), None),
        )
        .unwrap()
}

fn role_identities_do_not_share(fixture: &Fixture, transport: &Transport) {
    let one = format!("aws_role_name: '{TARGET}', aws_session_name: one");
    let two = format!("aws_role_name: '{TARGET}', aws_session_name: two");
    let first = resolve_one(fixture, &one);
    let second = resolve_one(fixture, &two);
    assert_eq!(first.access_key_id.expose_secret(), "ASIAONE");
    assert_eq!(second.access_key_id.expose_secret(), "ASIATWO");
    assert_eq!(first.secret_access_key.expose_secret(), "role-secret");
    assert_eq!(transport.record.count(), 4);
    let repeated = resolve_one(fixture, &one);
    assert_eq!(repeated.access_key_id.expose_secret(), "ASIAONE");
    assert_eq!(transport.record.count(), 4);
}

fn s3_pair_is_its_own_identity(fixture: &Fixture) {
    let (settings, s3) = load(
        "s3_access_key_id: s3-access, s3_secret_access_key: s3-secret, aws_region_name: us-east-1",
        AMBIENT,
    );
    let pair = fixture
        .runtime
        .block_on(
            fixture
                .resolver()
                .resolve_s3_credentials(&s3, &settings, None),
        )
        .unwrap();
    assert_eq!(pair.access_key_id.expose_secret(), "s3-access");
    assert_eq!(pair.secret_access_key.expose_secret(), "s3-secret");
    assert!(pair.session_token.is_none());
    let ambient = fixture
        .runtime
        .block_on(fixture.resolver().get_credentials(&settings, None))
        .unwrap();
    assert_eq!(ambient.access_key_id.expose_secret(), "ambient");
    assert_ne!(
        pair.access_key_id.expose_secret(),
        ambient.access_key_id.expose_secret()
    );
}

#[test]
fn bedrock_credentials_credential_identity_isolation() {
    let transport = Transport::with(&[
        ("GetCallerIdentity", vec![DENIED_IDENTITY, DENIED_IDENTITY]),
        (
            "AssumeRole",
            vec![role_reply("ASIAONE"), role_reply("ASIATWO")],
        ),
    ]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    role_identities_do_not_share(&fixture, &transport);
    s3_pair_is_its_own_identity(&fixture);
}

fn denied_transport() -> Transport {
    Transport::with(&[
        (
            "AssumeRole",
            vec![Reply::Error {
                status: 403,
                code: "AccessDenied",
                message: "the caller may not assume the role",
            }],
        ),
        (
            "GetCallerIdentity",
            vec![Reply::Identity {
                arn: "arn:aws:sts::999999999999:assumed-role/TargetRole/session",
            }],
        ),
    ])
}

fn failure_with_secrets(fixture: &Fixture) -> CredentialError {
    let access_key = SecretString::from("AKIAFIXTURESECRET".to_owned());
    let secret_key = SecretString::from("fixture-secret-never-log".to_owned());
    let auth = RoleAuth {
        role_name: TARGET,
        session_name: "bedrock",
        access_key_id: Some(&access_key),
        secret_access_key: Some(&secret_key),
        session_token: None,
        external_id: None,
        irsa_role_arn: None,
        web_identity_token_file: None,
        tags: None,
    };
    fixture
        .runtime
        .block_on(auth_with_aws_role(&fixture.resolver(), &auth))
        .unwrap_err()
}

fn assert_no_fixture_secret(rendered: &str) {
    assert!(!rendered.contains("AKIAFIXTURESECRET"));
    assert!(!rendered.contains("fixture-secret-never-log"));
    assert!(!rendered.contains("ambient-secret"));
}

#[test]
fn bedrock_credentials_secret_safe_failure() {
    let transport = denied_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let error = failure_with_secrets(&fixture);
    assert_no_fixture_secret(&format!("{error} {error:?}"));
    let successful = settings(
        "aws_access_key_id: AKIAFIXTURESECRET, aws_secret_access_key: fixture-secret-never-log, aws_region_name: us-east-1",
        AMBIENT,
    );
    let resolved = fixture
        .runtime
        .block_on(fixture.resolver().get_credentials(&successful, None))
        .unwrap();
    let rendered = format!("{resolved:?}");
    assert_no_fixture_secret(&rendered);
    assert!(rendered.contains("REDACTED"));
}

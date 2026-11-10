#![cfg(test)]

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::VerifySetting;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::Level;

use crate::aws_settings::AwsSessionTag;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::credentials::role::aws_role::auth_with_aws_role;
use crate::credentials::role::{CredentialTtl, RoleAuth, RoleCredentials};

use super::support::{Fixture, Reply, Transport, load, logged, settings};

const TARGET: &str = "arn:aws:iam::123456789012:role/TargetRole";
const OTHER: &str = "arn:aws:iam::999999999999:role/TargetRole";
const IRSA: &str = "arn:aws:iam::123456789012:role/IrsaRole";

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

fn web_identity_reply(access: &'static str) -> Reply {
    Reply::Credentials {
        access,
        secret: "web-secret",
        token: "web-token",
        expiration: "2026-01-01T01:00:00Z",
    }
}

fn resolve(
    fixture: &Fixture,
    settings: &AwsSettings,
    ssl_verify: Option<&VerifySetting>,
) -> Result<Credentials, CredentialError> {
    fixture
        .runtime
        .block_on(fixture.resolver().get_credentials(settings, ssl_verify))
}

fn run_role(fixture: &Fixture, auth: &RoleAuth<'_>) -> Result<RoleCredentials, CredentialError> {
    fixture
        .runtime
        .block_on(auth_with_aws_role(&fixture.resolver(), auth))
}

fn irsa_auth<'text>(
    role: &'text str,
    session: &'text str,
    key: Option<&'text SecretString>,
) -> RoleAuth<'text> {
    RoleAuth {
        role_name: role,
        session_name: session,
        access_key_id: key,
        secret_access_key: None,
        session_token: None,
        external_id: None,
        irsa_role_arn: Some(IRSA),
        web_identity_token_file: Some("/var/run/secrets/token"),
        tags: None,
    }
}

fn direct_auth<'text>(
    role: &'text str,
    session: &'text str,
    token: Option<&'text SecretString>,
) -> RoleAuth<'text> {
    RoleAuth {
        role_name: role,
        session_name: session,
        access_key_id: None,
        secret_access_key: None,
        session_token: token,
        external_id: None,
        irsa_role_arn: None,
        web_identity_token_file: None,
        tags: None,
    }
}

#[derive(Debug, Clone, Copy)]
enum Scenario {
    WebIdentityBeforeRole,
    RoleBeforeProfile,
    ProfileBeforeStatic,
    S3StaticPairIsolated,
    CredentialsCacheKeyIsolation,
    FixedEnvFallbackOnly,
    IrsaBranchAndFailure,
    NonIrsaAssumeRole,
    AccessDeniedSameIdentityFallback,
}

#[rstest]
#[case::web_identity_before_role(Scenario::WebIdentityBeforeRole)]
#[case::role_before_profile(Scenario::RoleBeforeProfile)]
#[case::profile_before_static(Scenario::ProfileBeforeStatic)]
#[case::s3_static_pair_isolated(Scenario::S3StaticPairIsolated)]
#[case::credentials_cache_key_isolation(Scenario::CredentialsCacheKeyIsolation)]
#[case::fixed_env_fallback_only(Scenario::FixedEnvFallbackOnly)]
#[case::irsa_branch_and_failure(Scenario::IrsaBranchAndFailure)]
#[case::non_irsa_assume_role(Scenario::NonIrsaAssumeRole)]
#[case::access_denied_same_identity_fallback(Scenario::AccessDeniedSameIdentityFallback)]
fn credentials_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::WebIdentityBeforeRole => {
            web_identity_wins_over_role();
            missing_session_shifts_to_role();
            empty_session_still_selects_web_identity();
        }
        Scenario::RoleBeforeProfile => {
            role_wins_over_profile_and_static();
            already_running_as_role_uses_ambient();
        }
        Scenario::ProfileBeforeStatic => {
            profile_wins_over_static();
            session_token_triple_is_direct();
            incomplete_static_falls_through_to_ambient();
        }
        Scenario::S3StaticPairIsolated => {
            s3_pair_bypasses_resolution();
            incomplete_s3_pair_resolves_normally();
        }
        Scenario::CredentialsCacheKeyIsolation => {
            identical_identity_hits_cache();
            profile_name_isolates_cache();
            ssl_verify_isolates_cache();
        }
        Scenario::FixedEnvFallbackOnly => {
            explicit_values_win_over_environment();
            role_environment_fields_reach_assume_role();
            web_identity_environment_fields_reach_sts();
        }
        Scenario::IrsaBranchAndFailure => {
            same_account_irsa_returns_whole_ttl();
            cross_account_irsa_uses_token_file();
            irsa_denial_logs_trust_policy();
            explicit_key_skips_irsa();
        }
        Scenario::NonIrsaAssumeRole => {
            explicit_keys_build_the_sts_client();
            default_client_and_fractional_ttl();
            negative_ttl_is_not_clamped();
        }
        Scenario::AccessDeniedSameIdentityFallback => {
            same_identity_falls_back_to_ambient();
            different_identity_reraises();
            other_errors_skip_identity_check();
        }
    }
}

fn web_identity_transport() -> Transport {
    Transport::with(&[
        (
            "AssumeRoleWithWebIdentity",
            vec![
                web_identity_reply("ASIAWEB"),
                web_identity_reply("ASIAEMPTY"),
            ],
        ),
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        ("AssumeRole", vec![role_reply("ASIAROLE")]),
    ])
}

fn web_identity_wins_over_role() {
    let transport = web_identity_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, Some("resolved-token"));
    let params = format!(
        "aws_role_name: '{TARGET}', aws_session_name: bedrock, aws_web_identity_token: web-token-ref"
    );
    let first = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(first.access_key_id.expose_secret(), "ASIAWEB");
    assert_eq!(transport.record.actions(), ["AssumeRoleWithWebIdentity"]);
    let repeated = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(repeated.access_key_id.expose_secret(), "ASIAWEB");
    assert_eq!(transport.record.count(), 1);
}

fn missing_session_shifts_to_role() {
    let transport = web_identity_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, Some("resolved-token"));
    let params = format!("aws_role_name: '{TARGET}', aws_web_identity_token: web-token-ref");
    let resolved = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ASIAROLE");
    assert_eq!(
        transport.record.actions(),
        ["GetCallerIdentity", "AssumeRole"]
    );
}

fn empty_session_still_selects_web_identity() {
    let transport = web_identity_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, Some("resolved-token"));
    let params = format!(
        "aws_role_name: '{TARGET}', aws_session_name: '', aws_web_identity_token: web-token-ref"
    );
    let resolved = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ASIAEMPTY");
    let calls = transport.record.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].value("RoleSessionName"), "");
}

fn role_transport() -> Transport {
    Transport::with(&[
        (
            "GetCallerIdentity",
            vec![
                DENIED_IDENTITY,
                DENIED_IDENTITY,
                Reply::Identity {
                    arn: "arn:aws:sts::123456789012:assumed-role/TargetRole/session",
                },
            ],
        ),
        (
            "AssumeRole",
            vec![role_reply("ASIAFIRST"), role_reply("ASIAEXPLICIT")],
        ),
    ])
}

fn role_wins_over_profile_and_static() {
    let transport = role_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params = format!(
        "aws_role_name: '{TARGET}', aws_profile_name: named, aws_access_key_id: static-key, aws_secret_access_key: static-secret, aws_region_name: us-east-1"
    );
    let first = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(first.access_key_id.expose_secret(), "ASIAFIRST");
    let calls = transport.record.calls();
    assert_eq!(
        calls[1].value("RoleSessionName"),
        "litellm-session-1767225600"
    );
    assert_eq!(calls[1].value("RoleArn"), TARGET);
    let repeated = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(repeated.access_key_id.expose_secret(), "ASIAFIRST");
    assert_eq!(transport.record.count(), 2);
    let explicit = format!("{params}, aws_session_name: explicit");
    let resolved = resolve(&fixture, &settings(&explicit, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ASIAEXPLICIT");
    let latest = transport.record.calls();
    assert_eq!(latest.len(), 4);
    assert_eq!(latest[3].value("RoleSessionName"), "explicit");
}

fn already_running_as_role_uses_ambient() {
    let transport = Transport::with(&[(
        "GetCallerIdentity",
        vec![Reply::Identity {
            arn: "arn:aws:sts::123456789012:assumed-role/TargetRole/session",
        }],
    )]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params = format!("aws_role_name: '{TARGET}', aws_session_name: other");
    let resolved = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ambient");
    assert!(resolved.session_token.is_none());
    assert_eq!(transport.record.actions(), ["GetCallerIdentity"]);
}

fn profile_environment() -> [(&'static str, &'static str); 5] {
    [
        ("AWS_ACCESS_KEY_ID", "ambient"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_SHARED_CREDENTIALS_FILE", "/credentials"),
        ("AWS_CONFIG_FILE", "/config"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ]
}

fn profile_files() -> [(&'static str, &'static str); 2] {
    [
        (
            "/credentials",
            "[named]\naws_access_key_id = profile\naws_secret_access_key = private\n",
        ),
        ("/config", ""),
    ]
}

fn profile_wins_over_static() {
    let transport = Transport::with(&[]);
    let environment = profile_environment();
    let fixture = Fixture::new(&environment, &profile_files(), &transport, None);
    let params = "aws_profile_name: named, aws_access_key_id: ignored, aws_secret_access_key: ignored, aws_session_token: ignored, aws_region_name: us-east-1";
    let resolved = resolve(&fixture, &settings(params, &environment), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "profile");
    assert_eq!(resolved.secret_access_key.expose_secret(), "private");
    assert!(resolved.session_token.is_none());
    assert_eq!(transport.record.count(), 0);
}

fn session_token_triple_is_direct() {
    let transport = Transport::with(&[]);
    let environment = profile_environment();
    let fixture = Fixture::new(&environment, &profile_files(), &transport, None);
    let params =
        "aws_access_key_id: session, aws_secret_access_key: secret, aws_session_token: token";
    let resolved = resolve(&fixture, &settings(params, &environment), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "session");
    assert_eq!(
        resolved
            .session_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("token")
    );
    assert_eq!(transport.record.count(), 0);
}

fn incomplete_static_falls_through_to_ambient() {
    let transport = Transport::with(&[]);
    let environment = profile_environment();
    let fixture = Fixture::new(&environment, &profile_files(), &transport, None);
    let statics =
        "aws_access_key_id: static, aws_secret_access_key: secret, aws_region_name: us-west-2";
    let resolved = resolve(&fixture, &settings(statics, &environment), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "static");
    let incomplete = "aws_access_key_id: static-only";
    let ambient = resolve(&fixture, &settings(incomplete, &environment), None).unwrap();
    assert_eq!(ambient.access_key_id.expose_secret(), "ambient");
    assert_eq!(transport.record.count(), 0);
}

fn s3_pair_bypasses_resolution() {
    let transport = Transport::with(&[]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params = format!(
        "s3_access_key_id: s3-access, s3_secret_access_key: s3-secret, aws_region_name: us-east-1, aws_role_name: '{TARGET}'"
    );
    let (settings, s3) = load(&params, AMBIENT);
    let resolved = fixture
        .runtime
        .block_on(
            fixture
                .resolver()
                .resolve_s3_credentials(&s3, &settings, None),
        )
        .unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "s3-access");
    assert_eq!(resolved.secret_access_key.expose_secret(), "s3-secret");
    assert!(resolved.session_token.is_none());
    assert_eq!(transport.record.count(), 0);
}

fn incomplete_s3_pair_resolves_normally() {
    let transport = Transport::with(&[]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let (settings, s3) = load(
        "s3_access_key_id: '', s3_secret_access_key: s3-secret, aws_access_key_id: static, aws_secret_access_key: static-secret, aws_region_name: us-east-1",
        AMBIENT,
    );
    let resolved = fixture
        .runtime
        .block_on(
            fixture
                .resolver()
                .resolve_s3_credentials(&s3, &settings, None),
        )
        .unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "static");
    assert_eq!(transport.record.count(), 0);
}

fn isolation_transport() -> Transport {
    Transport::with(&[
        (
            "GetCallerIdentity",
            vec![DENIED_IDENTITY, DENIED_IDENTITY, DENIED_IDENTITY],
        ),
        (
            "AssumeRole",
            vec![
                role_reply("ASIAONE"),
                role_reply("ASIATWO"),
                role_reply("ASIATHREE"),
            ],
        ),
    ])
}

fn identical_identity_hits_cache() {
    let transport = isolation_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params = format!("aws_role_name: '{TARGET}', aws_session_name: one");
    let first = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(first.access_key_id.expose_secret(), "ASIAONE");
    let repeated = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(repeated.access_key_id.expose_secret(), "ASIAONE");
    assert_eq!(transport.record.count(), 2);
}

fn profile_name_isolates_cache() {
    let transport = isolation_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params =
        format!("aws_role_name: '{TARGET}', aws_session_name: one, aws_profile_name: another");
    let resolved = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ASIATWO");
    assert_eq!(transport.record.count(), 2);
}

fn ssl_verify_isolates_cache() {
    let transport = isolation_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params = format!("aws_role_name: '{TARGET}', aws_session_name: one");
    let verified = resolve(
        &fixture,
        &settings(&params, AMBIENT),
        Some(&VerifySetting::Bool(true)),
    )
    .unwrap();
    assert_eq!(verified.access_key_id.expose_secret(), "ASIATHREE");
    let repeated = resolve(
        &fixture,
        &settings(&params, AMBIENT),
        Some(&VerifySetting::Bool(true)),
    )
    .unwrap();
    assert_eq!(repeated.access_key_id.expose_secret(), "ASIATHREE");
    assert_eq!(transport.record.count(), 4);
}

fn explicit_values_win_over_environment() {
    let environment = [
        ("AWS_ACCESS_KEY_ID", "env-access"),
        ("AWS_SECRET_ACCESS_KEY", "env-secret"),
        ("AWS_SESSION_TOKEN", "env-token"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let fixture = Fixture::new(&environment, &[], &Transport::with(&[]), None);
    let params = "aws_access_key_id: explicit, aws_secret_access_key: ''";
    let resolved = resolve(&fixture, &settings(params, &environment), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "explicit");
    assert_eq!(resolved.secret_access_key.expose_secret(), "");
    assert_eq!(
        resolved
            .session_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("env-token")
    );
}

fn role_environment_fields_reach_assume_role() {
    let environment = [
        ("AWS_ACCESS_KEY_ID", "env-access"),
        ("AWS_SECRET_ACCESS_KEY", "env-secret"),
        ("AWS_ROLE_NAME", TARGET),
        ("AWS_SESSION_NAME", "env-session"),
        ("AWS_EXTERNAL_ID", "env-external"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let transport = Transport::with(&[
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        ("AssumeRole", vec![role_reply("ASIAROLE")]),
    ]);
    let fixture = Fixture::new(&environment, &[], &transport, None);
    let resolved = resolve(&fixture, &settings("", &environment), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ASIAROLE");
    let calls = transport.record.calls();
    assert_eq!(calls[1].value("RoleSessionName"), "env-session");
    assert_eq!(calls[1].value("ExternalId"), "env-external");
}

fn web_identity_environment_fields_reach_sts() {
    let environment = [
        ("AWS_ACCESS_KEY_ID", "env-access"),
        ("AWS_SECRET_ACCESS_KEY", "env-secret"),
        ("AWS_ROLE_NAME", TARGET),
        ("AWS_SESSION_NAME", "env-session"),
        ("AWS_WEB_IDENTITY_TOKEN", "env-web-token"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let transport = Transport::with(&[(
        "AssumeRoleWithWebIdentity",
        vec![web_identity_reply("ASIAENV")],
    )]);
    let fixture = Fixture::new(&environment, &[], &transport, Some("resolved-token"));
    let resolved = resolve(&fixture, &settings("", &environment), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ASIAENV");
    let calls = transport.record.calls();
    assert_eq!(calls[0].value("RoleArn"), TARGET);
    assert_eq!(calls[0].value("RoleSessionName"), "env-session");
}

fn irsa_environment() -> [(&'static str, &'static str); 5] {
    [
        ("AWS_ROLE_ARN", IRSA),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/secrets/token"),
        ("AWS_ACCESS_KEY_ID", "ambient"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ]
}

fn irsa_files() -> [(&'static str, &'static str); 1] {
    [("/var/run/secrets/token", "irsa-token\n")]
}

fn irsa_reply(access: &'static str, secret: &'static str) -> Reply {
    Reply::Credentials {
        access,
        secret,
        token: "session-token",
        expiration: "2026-01-01T01:00:00Z",
    }
}

fn reply_error_denied() -> Reply {
    Reply::Error {
        status: 403,
        code: "AccessDenied",
        message: "User: arn:aws:iam::123456789012:role/IrsaRole is not authorized to perform: sts:AssumeRole on resource: arn:aws:iam::123456789012:role/IrsaRole",
    }
}

fn same_account_irsa_returns_whole_ttl() {
    let transport = Transport::with(&[
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        ("AssumeRole", vec![irsa_reply("ASIASAME", "same-secret")]),
    ]);
    let fixture = Fixture::new(&irsa_environment(), &irsa_files(), &transport, None);
    let role = run_role(&fixture, &irsa_auth(IRSA, "bedrock", None)).unwrap();
    assert_eq!(role.credentials.access_key_id.expose_secret(), "ASIASAME");
    assert_eq!(role.ttl, Some(CredentialTtl::Whole(3540)));
    assert_eq!(
        transport.record.actions(),
        ["GetCallerIdentity", "AssumeRole"]
    );
}

fn cross_account_irsa_uses_token_file() {
    let transport = Transport::with(&[
        (
            "AssumeRoleWithWebIdentity",
            vec![irsa_reply("ASIAIRSA", "irsa-secret")],
        ),
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        ("AssumeRole", vec![irsa_reply("ASIACROSS", "cross-secret")]),
    ]);
    let fixture = Fixture::new(&irsa_environment(), &irsa_files(), &transport, None);
    let role = run_role(&fixture, &irsa_auth(OTHER, "bedrock", None)).unwrap();
    assert_eq!(role.credentials.access_key_id.expose_secret(), "ASIACROSS");
    let calls = transport.record.calls();
    assert_eq!(
        calls
            .iter()
            .map(|call| call.action.as_str())
            .collect::<Vec<_>>(),
        [
            "AssumeRoleWithWebIdentity",
            "GetCallerIdentity",
            "AssumeRole"
        ]
    );
    assert_eq!(calls[0].value("RoleArn"), IRSA);
    assert_eq!(calls[0].value("RoleSessionName"), "bedrock");
}

fn irsa_denial_logs_trust_policy() {
    let transport = Transport::with(&[
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        ("AssumeRole", vec![reply_error_denied()]),
    ]);
    let fixture = Fixture::new(&irsa_environment(), &irsa_files(), &transport, None);
    let (failed, entries) = logged(|| run_role(&fixture, &irsa_auth(IRSA, "bedrock", None)));
    assert!(matches!(failed, Err(CredentialError::WebIdentity(_))));
    assert!(entries.iter().any(|(level, message)| *level == Level::ERROR
        && message.contains("Access denied when trying to assume role")
        && message.contains(IRSA)
        && message.contains("trust policy")));
}

fn explicit_key_skips_irsa() {
    let transport = Transport::with(&[(
        "AssumeRole",
        vec![irsa_reply("ASIAEXPLICIT", "explicit-secret")],
    )]);
    let fixture = Fixture::new(&irsa_environment(), &irsa_files(), &transport, None);
    let key = SecretString::from("explicit".to_owned());
    let role = run_role(&fixture, &irsa_auth(IRSA, "bedrock", Some(&key))).unwrap();
    assert_eq!(
        role.credentials.access_key_id.expose_secret(),
        "ASIAEXPLICIT"
    );
    let calls = transport.record.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].action, "AssumeRole");
    assert_eq!(calls[0].access_key_id.as_deref(), Some("explicit"));
}

fn assume_reply(
    access: &'static str,
    secret: &'static str,
    token: &'static str,
    expiration: &'static str,
) -> Reply {
    Reply::Credentials {
        access,
        secret,
        token,
        expiration,
    }
}

fn explicit_keys_build_the_sts_client() {
    let reply = assume_reply(
        "ASIAEXPLICIT",
        "explicit-secret",
        "explicit-token",
        "2026-01-01T01:00:00Z",
    );
    let transport = Transport::with(&[("AssumeRole", vec![reply])]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let key = SecretString::from("explicit-key".to_owned());
    let secret = SecretString::from("explicit-secret".to_owned());
    let token = SecretString::from("explicit-token".to_owned());
    let tags = [AwsSessionTag {
        key: "team".to_owned(),
        value: "genai".to_owned(),
    }];
    let auth = RoleAuth {
        role_name: TARGET,
        session_name: "bedrock",
        access_key_id: Some(&key),
        secret_access_key: Some(&secret),
        session_token: Some(&token),
        external_id: Some("external-1"),
        irsa_role_arn: None,
        web_identity_token_file: None,
        tags: Some(&tags),
    };
    let role = run_role(&fixture, &auth).unwrap();
    assert_eq!(
        role.credentials.access_key_id.expose_secret(),
        "ASIAEXPLICIT"
    );
    assert_eq!(role.ttl, Some(CredentialTtl::Fractional(3540.0)));
    let calls = transport.record.calls();
    assert_eq!(calls[0].access_key_id.as_deref(), Some("explicit-key"));
    assert_eq!(calls[0].session_token.as_deref(), Some("explicit-token"));
    assert_eq!(calls[0].value("ExternalId"), "external-1");
    assert_eq!(calls[0].value("Tags.member.1.Key"), "team");
    assert_eq!(calls[0].value("Tags.member.1.Value"), "genai");
}

fn default_client_and_fractional_ttl() {
    let reply = assume_reply(
        "ASIADEFAULT",
        "default-secret",
        "default-token",
        "2026-01-01T01:00:00.5Z",
    );
    let transport = Transport::with(&[("AssumeRole", vec![reply])]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let role = run_role(&fixture, &direct_auth(TARGET, "bedrock", None)).unwrap();
    assert_eq!(
        role.credentials.access_key_id.expose_secret(),
        "ASIADEFAULT"
    );
    assert_eq!(role.ttl, Some(CredentialTtl::Fractional(3540.5)));
    assert_eq!(
        transport.record.calls()[0].access_key_id.as_deref(),
        Some("ambient")
    );
}

fn negative_ttl_is_not_clamped() {
    let transport = Transport::with(&[(
        "AssumeRole",
        vec![
            assume_reply(
                "ASIADEFAULT",
                "default-secret",
                "default-token",
                "2026-01-01T01:00:00Z",
            ),
            assume_reply(
                "ASIANEGATIVE",
                "negative-secret",
                "negative-token",
                "2025-12-31T23:59:00Z",
            ),
        ],
    )]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let first = run_role(&fixture, &direct_auth(TARGET, "bedrock", None)).unwrap();
    assert_eq!(
        first.credentials.access_key_id.expose_secret(),
        "ASIADEFAULT"
    );
    let second = run_role(&fixture, &direct_auth(TARGET, "bedrock", None)).unwrap();
    assert_eq!(
        second.credentials.access_key_id.expose_secret(),
        "ASIANEGATIVE"
    );
    assert_eq!(second.ttl, Some(CredentialTtl::Fractional(-120.0)));
}

fn denial_transport() -> Transport {
    Transport::with(&[
        (
            "AssumeRole",
            vec![
                Reply::Error {
                    status: 403,
                    code: "AccessDenied",
                    message: "the caller may not assume the role",
                },
                Reply::Error {
                    status: 403,
                    code: "AccessDenied",
                    message: "the caller may not assume the role",
                },
                Reply::Error {
                    status: 500,
                    code: "InternalFailure",
                    message: "the service failed",
                },
            ],
        ),
        (
            "GetCallerIdentity",
            vec![
                Reply::Identity {
                    arn: "arn:aws:sts::123456789012:assumed-role/TargetRole/session",
                },
                Reply::Identity {
                    arn: "arn:aws:sts::999999999999:assumed-role/TargetRole/session",
                },
            ],
        ),
    ])
}

fn same_identity_falls_back_to_ambient() {
    let transport = denial_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let (attempt, entries) = logged(|| run_role(&fixture, &direct_auth(TARGET, "bedrock", None)));
    let fallback = attempt.unwrap();
    assert_eq!(
        fallback.credentials.access_key_id.expose_secret(),
        "ambient"
    );
    assert_eq!(fallback.ttl, None);
    assert!(entries.iter().any(|(level, message)| *level == Level::WARN
        && message.contains("AssumeRole failed for arn:aws:iam::123456789012:role/TargetRole")
        && message.contains("falling back to ambient credentials")));
}

fn different_identity_reraises() {
    let transport = denial_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    run_role(&fixture, &direct_auth(TARGET, "bedrock", None)).unwrap();
    let (failed, entries) = logged(|| run_role(&fixture, &direct_auth(TARGET, "bedrock", None)));
    assert!(matches!(failed, Err(CredentialError::AssumeRole { .. })));
    assert!(entries.iter().any(|(level, message)| *level == Level::ERROR
        && message.contains("AssumeRole AccessDenied for arn:aws:iam::123456789012:role/TargetRole")
        && message.contains("NOT the same role")));
}

fn other_errors_skip_identity_check() {
    let transport = denial_transport();
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    run_role(&fixture, &direct_auth(TARGET, "bedrock", None)).unwrap();
    run_role(&fixture, &direct_auth(TARGET, "bedrock", None)).unwrap();
    let (failed, _entries) = logged(|| run_role(&fixture, &direct_auth(TARGET, "bedrock", None)));
    assert!(matches!(failed, Err(CredentialError::AssumeRole { .. })));
    let calls = transport.record.calls();
    assert_eq!(calls.len(), 5);
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.action == "GetCallerIdentity")
            .count(),
        2
    );
}

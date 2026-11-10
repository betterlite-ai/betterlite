#![cfg(test)]

use std::time::Duration;

use aws_types::os_shim_internal::Fs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::VerifySetting;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::Level;

use crate::aws_settings::AwsSessionTag;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::credentials::role::aws_role::auth_with_aws_role;
use crate::credentials::role::{
    CredentialTtl, RoleAuth, RoleCredentials, resolve_role_credentials,
};

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

const TARGET_IDENTITY: Reply = Reply::Identity {
    arn: "arn:aws:sts::123456789012:assumed-role/TargetRole/session",
};

fn reply(access: &'static str) -> Reply {
    Reply::Credentials {
        access,
        secret: "role-secret",
        token: "role-token",
        expiration: "2026-01-01T01:00:00Z",
    }
}

fn auth<'text>(
    role: &'text str,
    session: &'text str,
    key: Option<&'text SecretString>,
    irsa: bool,
) -> RoleAuth<'text> {
    RoleAuth {
        role_name: role,
        session_name: session,
        access_key_id: key,
        secret_access_key: None,
        session_token: None,
        external_id: None,
        irsa_role_arn: irsa.then_some(IRSA),
        web_identity_token_file: irsa.then_some("/var/run/secrets/token"),
        tags: None,
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
            role_credentials_expose_the_cache_ttls();
        }
        Scenario::ProfileBeforeStatic => {
            profile_wins_over_static();
            session_token_triple_is_direct();
            incomplete_static_falls_through_to_ambient();
            profile_credentials_bypass_the_cache();
            session_token_triple_bypasses_the_cache();
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
            environment_fields_reach_the_selected_branch();
        }
        Scenario::IrsaBranchAndFailure => {
            same_account_irsa_returns_whole_ttl();
            cross_account_irsa_uses_token_file();
            irsa_denial_logs_trust_policy();
            irsa_failure_without_trust_policy_text_stays_quiet();
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

fn transport(assume: Vec<Reply>, web: Vec<Reply>, identity: Vec<Reply>) -> Transport {
    Transport::with(&[
        ("AssumeRole", assume),
        ("AssumeRoleWithWebIdentity", web),
        ("GetCallerIdentity", identity),
    ])
}

fn web_identity_wins_over_role() {
    let transport = transport(
        vec![reply("ASIAROLE")],
        vec![reply("ASIAWEB"), reply("ASIAEMPTY")],
        vec![DENIED_IDENTITY],
    );
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
    let transport = transport(
        vec![reply("ASIAROLE")],
        vec![reply("ASIAWEB"), reply("ASIAEMPTY")],
        vec![DENIED_IDENTITY],
    );
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
    let transport = transport(vec![], vec![reply("ASIAEMPTY")], vec![]);
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

fn role_wins_over_profile_and_static() {
    let transport = transport(
        vec![reply("ASIAFIRST"), reply("ASIAEXPLICIT")],
        Vec::new(),
        vec![DENIED_IDENTITY, DENIED_IDENTITY, TARGET_IDENTITY],
    );
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
    let transport = Transport::with(&[("GetCallerIdentity", vec![TARGET_IDENTITY])]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params = format!("aws_role_name: '{TARGET}', aws_session_name: other");
    let resolved = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ambient");
    assert!(resolved.session_token.is_none());
    assert_eq!(transport.record.actions(), ["GetCallerIdentity"]);
}

fn role_credentials_expose_the_cache_ttls() {
    let transport = transport(
        vec![reply("ASIAROLE")],
        Vec::new(),
        vec![DENIED_IDENTITY, TARGET_IDENTITY],
    );
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let role_settings = settings(
        &format!("aws_role_name: '{TARGET}', aws_session_name: bedrock"),
        AMBIENT,
    );
    let resolved = fixture.runtime.block_on(resolve_role_credentials(
        &fixture.resolver(),
        &role_settings,
        TARGET,
        None,
    ));
    assert_eq!(resolved.unwrap().cache_ttl, Some(Duration::from_mins(59)));
    let already = fixture.runtime.block_on(resolve_role_credentials(
        &fixture.resolver(),
        &role_settings,
        TARGET,
        None,
    ));
    assert_eq!(already.unwrap().cache_ttl, None);
}

fn profile_wins_over_static() {
    let transport = Transport::with(&[]);
    let environment = [
        ("AWS_ACCESS_KEY_ID", "ambient"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_SHARED_CREDENTIALS_FILE", "/credentials"),
        ("AWS_CONFIG_FILE", "/config"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let files = [
        (
            "/credentials",
            "[named]\naws_access_key_id = profile\naws_secret_access_key = private\n",
        ),
        ("/config", ""),
    ];
    let fixture = Fixture::new(&environment, &files, &transport, None);
    let params = "aws_profile_name: named, aws_access_key_id: ignored, aws_secret_access_key: ignored, aws_session_token: ignored, aws_region_name: us-east-1";
    let resolved = resolve(&fixture, &settings(params, &environment), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "profile");
    assert_eq!(resolved.secret_access_key.expose_secret(), "private");
    assert!(resolved.session_token.is_none());
    assert_eq!(transport.record.count(), 0);
}

fn session_token_triple_is_direct() {
    let transport = Transport::with(&[]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params =
        "aws_access_key_id: session, aws_secret_access_key: secret, aws_session_token: token";
    let resolved = resolve(&fixture, &settings(params, AMBIENT), None).unwrap();
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
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let statics =
        "aws_access_key_id: static, aws_secret_access_key: secret, aws_region_name: us-west-2";
    let resolved = resolve(&fixture, &settings(statics, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "static");
    let incomplete = "aws_access_key_id: static-only";
    let ambient = resolve(&fixture, &settings(incomplete, AMBIENT), None).unwrap();
    assert_eq!(ambient.access_key_id.expose_secret(), "ambient");
    assert_eq!(transport.record.count(), 0);
}

fn profile_credentials_bypass_the_cache() {
    let transport = Transport::with(&[]);
    let environment = [
        ("AWS_SHARED_CREDENTIALS_FILE", "/credentials"),
        ("AWS_CONFIG_FILE", "/config"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let files = [
        (
            "/credentials",
            "[named]\naws_access_key_id = first\naws_secret_access_key = first-secret\n",
        ),
        ("/config", ""),
    ];
    let mut fixture = Fixture::new(&environment, &files, &transport, None);
    let params = "aws_profile_name: named, aws_region_name: us-east-1";
    let first = resolve(&fixture, &settings(params, &environment), None).unwrap();
    assert_eq!(first.access_key_id.expose_secret(), "first");
    assert!(fixture.cache_events().is_empty());
    let rotated = [
        (
            "/credentials",
            "[named]\naws_access_key_id = rotated\naws_secret_access_key = rotated-secret\n",
        ),
        ("/config", ""),
    ];
    fixture.native.filesystem = Fs::from_slice(&rotated);
    let second = resolve(&fixture, &settings(params, &environment), None).unwrap();
    assert_eq!(second.access_key_id.expose_secret(), "rotated");
}

fn session_token_triple_bypasses_the_cache() {
    let transport = Transport::with(&[]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let triple =
        "aws_access_key_id: session, aws_secret_access_key: secret, aws_session_token: token";
    let resolved = resolve(&fixture, &settings(triple, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "session");
    assert!(fixture.cache_events().is_empty());
    let statics =
        "aws_access_key_id: static, aws_secret_access_key: secret, aws_region_name: us-west-2";
    let cached = resolve(&fixture, &settings(statics, AMBIENT), None).unwrap();
    assert_eq!(cached.access_key_id.expose_secret(), "static");
    assert!(!fixture.cache_events().is_empty());
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
    assert_eq!(resolved.access_key_id.expose_secret(), "ambient");
    assert_eq!(resolved.secret_access_key.expose_secret(), "ambient-secret");
    assert_eq!(transport.record.count(), 0);
}

fn identical_identity_hits_cache() {
    let transport = transport(
        vec![reply("ASIAONE"), reply("ASIATWO"), reply("ASIATHREE")],
        Vec::new(),
        vec![DENIED_IDENTITY, DENIED_IDENTITY, DENIED_IDENTITY],
    );
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params = format!("aws_role_name: '{TARGET}', aws_session_name: one");
    let first = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(first.access_key_id.expose_secret(), "ASIAONE");
    let repeated = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(repeated.access_key_id.expose_secret(), "ASIAONE");
    assert_eq!(transport.record.count(), 2);
}

fn profile_name_isolates_cache() {
    let transport = transport(
        vec![reply("ASIAONE"), reply("ASIATWO"), reply("ASIATHREE")],
        Vec::new(),
        vec![DENIED_IDENTITY, DENIED_IDENTITY, DENIED_IDENTITY],
    );
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let base = format!("aws_role_name: '{TARGET}', aws_session_name: one");
    let first = resolve(&fixture, &settings(&base, AMBIENT), None).unwrap();
    assert_eq!(first.access_key_id.expose_secret(), "ASIAONE");
    let params =
        format!("aws_role_name: '{TARGET}', aws_session_name: one, aws_profile_name: another");
    let resolved = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ASIATWO");
    assert_eq!(transport.record.count(), 4);
}

fn ssl_verify_isolates_cache() {
    let transport = transport(
        vec![reply("ASIAONE"), reply("ASIATWO"), reply("ASIATHREE")],
        Vec::new(),
        vec![DENIED_IDENTITY, DENIED_IDENTITY, DENIED_IDENTITY],
    );
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let params = format!("aws_role_name: '{TARGET}', aws_session_name: one");
    let first = resolve(&fixture, &settings(&params, AMBIENT), None).unwrap();
    assert_eq!(first.access_key_id.expose_secret(), "ASIAONE");
    let verified = resolve(
        &fixture,
        &settings(&params, AMBIENT),
        Some(&VerifySetting::Bool(true)),
    )
    .unwrap();
    assert_eq!(verified.access_key_id.expose_secret(), "ASIATWO");
    let repeated = resolve(
        &fixture,
        &settings(&params, AMBIENT),
        Some(&VerifySetting::Bool(true)),
    )
    .unwrap();
    assert_eq!(repeated.access_key_id.expose_secret(), "ASIATWO");
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

fn environment_fields_reach_the_selected_branch() {
    let role_environment = [
        ("AWS_ACCESS_KEY_ID", "env-access"),
        ("AWS_SECRET_ACCESS_KEY", "env-secret"),
        ("AWS_ROLE_NAME", TARGET),
        ("AWS_SESSION_NAME", "env-session"),
        ("AWS_EXTERNAL_ID", "env-external"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let transport = Transport::with(&[
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        ("AssumeRole", vec![reply("ASIAROLE")]),
    ]);
    let fixture = Fixture::new(&role_environment, &[], &transport, None);
    let resolved = resolve(&fixture, &settings("", &role_environment), None).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "ASIAROLE");
    let calls = transport.record.calls();
    assert_eq!(calls[1].value("RoleSessionName"), "env-session");
    assert_eq!(calls[1].value("ExternalId"), "env-external");

    let identity_environment = [
        ("AWS_ACCESS_KEY_ID", "env-access"),
        ("AWS_SECRET_ACCESS_KEY", "env-secret"),
        ("AWS_ROLE_NAME", TARGET),
        ("AWS_SESSION_NAME", "env-session"),
        ("AWS_WEB_IDENTITY_TOKEN", "env-web-token"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let web_transport = Transport::with(&[("AssumeRoleWithWebIdentity", vec![reply("ASIAENV")])]);
    let web_fixture = Fixture::new(
        &identity_environment,
        &[],
        &web_transport,
        Some("resolved-token"),
    );
    let web_credentials =
        resolve(&web_fixture, &settings("", &identity_environment), None).unwrap();
    assert_eq!(web_credentials.access_key_id.expose_secret(), "ASIAENV");
    let web_calls = web_transport.record.calls();
    assert_eq!(web_calls[0].value("RoleArn"), TARGET);
    assert_eq!(web_calls[0].value("RoleSessionName"), "env-session");
}

fn same_account_irsa_returns_whole_ttl() {
    let transport = Transport::with(&[
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        ("AssumeRole", vec![reply("ASIASAME")]),
    ]);
    let environment = [
        ("AWS_ROLE_ARN", IRSA),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/secrets/token"),
        ("AWS_ACCESS_KEY_ID", "ambient"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let files = [("/var/run/secrets/token", "irsa-token\n")];
    let fixture = Fixture::new(&environment, &files, &transport, None);
    let role = run_role(&fixture, &auth(IRSA, "bedrock", None, true)).unwrap();
    assert_eq!(role.credentials.access_key_id.expose_secret(), "ASIASAME");
    assert_eq!(role.ttl, Some(CredentialTtl::Whole(3540)));
    assert_eq!(
        transport.record.actions(),
        ["GetCallerIdentity", "AssumeRole"]
    );
}

fn cross_account_irsa_uses_token_file() {
    let transport = Transport::with(&[
        ("AssumeRoleWithWebIdentity", vec![reply("ASIAIRSA")]),
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        ("AssumeRole", vec![reply("ASIACROSS")]),
    ]);
    let environment = [
        ("AWS_ROLE_ARN", IRSA),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/secrets/token"),
        ("AWS_ACCESS_KEY_ID", "ambient"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let files = [("/var/run/secrets/token", "irsa-token\n")];
    let fixture = Fixture::new(&environment, &files, &transport, None);
    let role = run_role(&fixture, &auth(OTHER, "bedrock", None, true)).unwrap();
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
        (
            "AssumeRole",
            vec![Reply::Error {
                status: 403,
                code: "AccessDenied",
                message: "User: arn:aws:iam::123456789012:role/IrsaRole is not authorized to perform: sts:AssumeRole on resource: arn:aws:iam::123456789012:role/IrsaRole",
            }],
        ),
    ]);
    let environment = [
        ("AWS_ROLE_ARN", IRSA),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/secrets/token"),
        ("AWS_ACCESS_KEY_ID", "ambient"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let files = [("/var/run/secrets/token", "irsa-token\n")];
    let fixture = Fixture::new(&environment, &files, &transport, None);
    let (failed, entries) = logged(|| run_role(&fixture, &auth(IRSA, "bedrock", None, true)));
    assert!(matches!(failed, Err(CredentialError::WebIdentity(_))));
    assert!(entries.iter().any(|(level, message)| *level == Level::ERROR
        && message.contains("Access denied when trying to assume role")
        && message.contains(IRSA)
        && message.contains("trust policy")));
}

fn irsa_failure_without_trust_policy_text_stays_quiet() {
    let transport = Transport::with(&[
        ("AssumeRoleWithWebIdentity", vec![reply("ASIAIRSA")]),
        ("GetCallerIdentity", vec![DENIED_IDENTITY]),
        (
            "AssumeRole",
            vec![denied("AccessDenied", "the caller may not assume the role")],
        ),
    ]);
    let files = [("/var/run/secrets/token", "irsa-token\n")];
    let environment = [
        ("AWS_ROLE_ARN", IRSA),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/secrets/token"),
        ("AWS_ACCESS_KEY_ID", "ambient"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let fixture = Fixture::new(&environment, &files, &transport, None);
    let (failed, entries) = logged(|| run_role(&fixture, &auth(OTHER, "bedrock", None, true)));
    assert!(matches!(failed, Err(CredentialError::WebIdentity(_))));
    assert!(entries.is_empty());
}

fn explicit_key_skips_irsa() {
    let transport = Transport::with(&[("AssumeRole", vec![reply("ASIAEXPLICIT")])]);
    let environment = [
        ("AWS_ROLE_ARN", IRSA),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/secrets/token"),
        ("AWS_ACCESS_KEY_ID", "ambient"),
        ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
        ("AWS_EC2_METADATA_DISABLED", "true"),
    ];
    let files = [("/var/run/secrets/token", "irsa-token\n")];
    let fixture = Fixture::new(&environment, &files, &transport, None);
    let key = SecretString::from("explicit".to_owned());
    let role = run_role(&fixture, &auth(IRSA, "bedrock", Some(&key), true)).unwrap();
    assert_eq!(
        role.credentials.access_key_id.expose_secret(),
        "ASIAEXPLICIT"
    );
    let calls = transport.record.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].action, "AssumeRole");
    assert_eq!(calls[0].access_key_id.as_deref(), Some("explicit"));
}

fn explicit_keys_build_the_sts_client() {
    let transport = Transport::with(&[(
        "AssumeRole",
        vec![Reply::Credentials {
            access: "ASIAEXPLICIT",
            secret: "explicit-secret",
            token: "explicit-token",
            expiration: "2026-01-01T01:00:00Z",
        }],
    )]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let key = SecretString::from("explicit-key".to_owned());
    let secret = SecretString::from("explicit-secret".to_owned());
    let token = SecretString::from("explicit-token".to_owned());
    let tags = [AwsSessionTag {
        key: "team".to_owned(),
        value: "genai".to_owned(),
    }];
    let mut role_auth = auth(TARGET, "bedrock", Some(&key), false);
    role_auth.secret_access_key = Some(&secret);
    role_auth.session_token = Some(&token);
    role_auth.external_id = Some("external-1");
    role_auth.tags = Some(&tags);
    let role = run_role(&fixture, &role_auth).unwrap();
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
    let transport = Transport::with(&[(
        "AssumeRole",
        vec![Reply::Credentials {
            access: "ASIADEFAULT",
            secret: "default-secret",
            token: "default-token",
            expiration: "2026-01-01T01:00:00.5Z",
        }],
    )]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let role = run_role(&fixture, &auth(TARGET, "bedrock", None, false)).unwrap();
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
            Reply::Credentials {
                access: "ASIADEFAULT",
                secret: "default-secret",
                token: "default-token",
                expiration: "2026-01-01T01:00:00Z",
            },
            Reply::Credentials {
                access: "ASIANEGATIVE",
                secret: "negative-secret",
                token: "negative-token",
                expiration: "2025-12-31T23:59:00Z",
            },
        ],
    )]);
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let first = run_role(&fixture, &auth(TARGET, "bedrock", None, false)).unwrap();
    assert_eq!(
        first.credentials.access_key_id.expose_secret(),
        "ASIADEFAULT"
    );
    let second = run_role(&fixture, &auth(TARGET, "bedrock", None, false)).unwrap();
    assert_eq!(
        second.credentials.access_key_id.expose_secret(),
        "ASIANEGATIVE"
    );
    assert_eq!(second.ttl, Some(CredentialTtl::Fractional(-120.0)));
}

fn denied(error_code: &'static str, message: &'static str) -> Reply {
    Reply::Error {
        status: 403,
        code: error_code,
        message,
    }
}

fn same_identity_falls_back_to_ambient() {
    let transport = transport(
        vec![
            denied("AccessDenied", "the caller may not assume the role"),
            denied("AccessDenied", "the caller may not assume the role"),
            Reply::Error {
                status: 500,
                code: "InternalFailure",
                message: "the service failed",
            },
        ],
        Vec::new(),
        vec![
            TARGET_IDENTITY,
            Reply::Identity {
                arn: "arn:aws:sts::999999999999:assumed-role/TargetRole/session",
            },
        ],
    );
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    let (attempt, entries) = logged(|| run_role(&fixture, &auth(TARGET, "bedrock", None, false)));
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
    let transport = transport(
        vec![
            denied("AccessDenied", "the caller may not assume the role"),
            denied("AccessDenied", "the caller may not assume the role"),
            Reply::Error {
                status: 500,
                code: "InternalFailure",
                message: "the service failed",
            },
        ],
        Vec::new(),
        vec![
            TARGET_IDENTITY,
            Reply::Identity {
                arn: "arn:aws:sts::999999999999:assumed-role/TargetRole/session",
            },
        ],
    );
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    run_role(&fixture, &auth(TARGET, "bedrock", None, false)).unwrap();
    let (failed, entries) = logged(|| run_role(&fixture, &auth(TARGET, "bedrock", None, false)));
    assert!(matches!(failed, Err(CredentialError::AssumeRole { .. })));
    assert!(entries.iter().any(|(level, message)| *level == Level::ERROR
        && message.contains("AssumeRole AccessDenied for arn:aws:iam::123456789012:role/TargetRole")
        && message.contains("NOT the same role")));
}

fn other_errors_skip_identity_check() {
    let transport = transport(
        vec![
            denied("AccessDenied", "the caller may not assume the role"),
            denied("AccessDenied", "the caller may not assume the role"),
            Reply::Error {
                status: 500,
                code: "InternalFailure",
                message: "the service failed",
            },
        ],
        Vec::new(),
        vec![TARGET_IDENTITY, TARGET_IDENTITY],
    );
    let fixture = Fixture::new(AMBIENT, &[], &transport, None);
    run_role(&fixture, &auth(TARGET, "bedrock", None, false)).unwrap();
    run_role(&fixture, &auth(TARGET, "bedrock", None, false)).unwrap();
    let (failed, _entries) = logged(|| run_role(&fixture, &auth(TARGET, "bedrock", None, false)));
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

use std::collections::BTreeMap;
use std::sync::PoisonError;

use secrecy::ExposeSecret as _;

use super::fixture::{self, Answer, Exchange};
use crate::assume_role::expiry::{Expiration, StsResponse};
use crate::aws_endpoints::sts::StsClientKwargs;
use crate::aws_settings::AwsSessionTag;
use crate::web_identity::error::WebIdentityError;
use crate::web_identity::irsa::{
    CrossAccountIrsa, SameAccountIrsa, handle_irsa_cross_account, handle_irsa_same_account,
};

const ENVIRONMENT: [(&str, &str); 4] = [
    ("AWS_ACCESS_KEY_ID", "AKIAAMBIENT"),
    ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
    ("AWS_SESSION_TOKEN", "ambient-session"),
    ("AWS_REGION", "us-east-1"),
];
const ROLE_ARN: &str = "arn:aws:iam::123456789012:role/TargetRole";
const IRSA_ROLE_ARN: &str = "arn:aws:iam::123456789012:role/IrsaRole";
const SESSION_NAME: &str = "bedrock-session";
const ENDPOINT: &str = "https://sts.us-east-1.amazonaws.com";
const REGION: &str = "us-east-1";
const TOKEN_FILE: &str = "/var/run/secrets/token";
const IRSA_TOKEN: &str = "irsa-token";
const TARGET_EXPIRATION: &str = "2026-01-01T01:00:00Z";
const TARGET_EXPIRATION_EPOCH: i64 = 1_767_229_200;

fn tags() -> Vec<AwsSessionTag> {
    vec![
        AwsSessionTag {
            key: "env".to_owned(),
            value: "prod".to_owned(),
        },
        AwsSessionTag {
            key: "team".to_owned(),
            value: "genai".to_owned(),
        },
    ]
}

fn replies<const N: usize>(pairs: [(&str, Answer); N]) -> BTreeMap<String, Answer> {
    pairs
        .into_iter()
        .map(|(action, answer)| (action.to_owned(), answer))
        .collect()
}

fn calls(recorder: &fixture::Recorder) -> Vec<Exchange> {
    recorder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn form<'exchange>(exchange: &'exchange Exchange, name: &str) -> Option<&'exchange str> {
    exchange.form.get(name).map(String::as_str)
}

fn tags_of(exchange: &Exchange) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for (name, value) in &exchange.form {
        let Some(rest) = name.strip_prefix("Tags.member.") else {
            continue;
        };
        let Some((index, field)) = rest.split_once('.') else {
            continue;
        };
        let Ok(position) = index.parse::<usize>() else {
            continue;
        };
        while found.len() < position {
            found.push((String::new(), String::new()));
        }
        if let Some(entry) = found.get_mut(position.saturating_sub(1)) {
            match field {
                "Key" => entry.0 = value.clone(),
                "Value" => entry.1 = value.clone(),
                _ => {}
            }
        }
    }
    found
}

fn expected_tags() -> [(String, String); 2] {
    [
        ("env".to_owned(), "prod".to_owned()),
        ("team".to_owned(), "genai".to_owned()),
    ]
}

fn assert_target(response: &StsResponse) {
    assert_eq!(
        response.credentials.access_key_id.expose_secret(),
        "ASIATARGET"
    );
    assert_eq!(
        response.credentials.secret_access_key.expose_secret(),
        "target-secret"
    );
    assert_expiration(response);
}

fn assert_expiration(response: &StsResponse) {
    let Expiration::Aware(expiry) = &response.credentials.expiration else {
        panic!("the fixture expiration is timezone aware");
    };
    assert_eq!(expiry.timestamp(), TARGET_EXPIRATION_EPOCH);
}

pub(super) fn same_account_irsa() {
    let tags = tags();
    let input = SameAccountIrsa {
        role_arn: ROLE_ARN,
        session_name: SESSION_NAME,
        external_id: Some("external-1"),
        tags: Some(tags.as_slice()),
    };
    let (response, exchanges) = run_same_account(
        &input,
        replies([
            (
                "GetCallerIdentity",
                fixture::identity_ok("arn:aws:sts::123456789012:assumed-role/Current/session"),
            ),
            (
                "AssumeRole",
                fixture::assume_role_ok(
                    "ASIATARGET",
                    "target-secret",
                    "target-session",
                    TARGET_EXPIRATION,
                ),
            ),
        ]),
    );
    assert_target(&response.unwrap());
    assert_same_account_calls(&exchanges);
    assert_same_account_without_options();
    assert_same_account_error();
}

fn run_same_account(
    input: &SameAccountIrsa<'_>,
    replies: BTreeMap<String, Answer>,
) -> (Result<StsResponse, WebIdentityError>, Vec<Exchange>) {
    let (native, recorder) = fixture::sources(&ENVIRONMENT, &[], replies);
    let kwargs = fixture::kwargs(&ENVIRONMENT, Some(ENDPOINT), Some(REGION));
    let result = fixture::runtime().block_on(handle_irsa_same_account(input, &kwargs, &native));
    (result, calls(&recorder))
}

fn assert_same_account_calls(exchanges: &[Exchange]) {
    assert_eq!(exchanges.len(), 2_usize);
    assert_eq!(exchanges[0].action, "GetCallerIdentity");
    assert_eq!(exchanges[0].access_key.as_deref(), Some("AKIAAMBIENT"));
    let assume = &exchanges[1];
    assert_eq!(assume.action, "AssumeRole");
    assert_eq!(form(assume, "RoleArn"), Some(ROLE_ARN));
    assert_eq!(form(assume, "RoleSessionName"), Some(SESSION_NAME));
    assert_eq!(form(assume, "ExternalId"), Some("external-1"));
    assert_eq!(tags_of(assume), expected_tags());
    assert_eq!(assume.access_key.as_deref(), Some("AKIAAMBIENT"));
    assert_eq!(assume.region.as_deref(), Some(REGION));
    assert_eq!(assume.host.as_deref(), Some("sts.us-east-1.amazonaws.com"));
}

fn assert_same_account_without_options() {
    let input = SameAccountIrsa {
        role_arn: ROLE_ARN,
        session_name: SESSION_NAME,
        external_id: None,
        tags: None,
    };
    let (response, exchanges) = run_same_account(
        &input,
        replies([
            ("GetCallerIdentity", fixture::access_denied()),
            (
                "AssumeRole",
                fixture::assume_role_ok("ASIATARGET", "s", "t", TARGET_EXPIRATION),
            ),
        ]),
    );
    response.unwrap();
    assert_eq!(exchanges.len(), 2_usize);
    assert_eq!(exchanges[0].action, "GetCallerIdentity");
    let assume = &exchanges[1];
    assert_eq!(form(assume, "ExternalId"), None);
    assert!(assume.form.keys().all(|name| !name.starts_with("Tags")));
}

fn assert_same_account_error() {
    let input = SameAccountIrsa {
        role_arn: ROLE_ARN,
        session_name: SESSION_NAME,
        external_id: None,
        tags: None,
    };
    let (response, _exchanges) = run_same_account(
        &input,
        replies([
            (
                "GetCallerIdentity",
                fixture::identity_ok("arn:aws:sts::1:assumed-role/R/s"),
            ),
            ("AssumeRole", fixture::access_denied()),
        ]),
    );
    assert!(matches!(
        response.unwrap_err(),
        WebIdentityError::AssumeRole { .. }
    ));
    assert_same_account_missing_members();
}

fn assert_same_account_missing_members() {
    for member in [
        "AccessKeyId",
        "SecretAccessKey",
        "SessionToken",
        "Expiration",
    ] {
        let input = SameAccountIrsa {
            role_arn: ROLE_ARN,
            session_name: SESSION_NAME,
            external_id: None,
            tags: None,
        };
        let (response, _exchanges) = run_same_account(
            &input,
            replies([
                ("GetCallerIdentity", fixture::access_denied()),
                ("AssumeRole", fixture::assume_role_missing(member)),
            ]),
        );
        fixture::assert_missing(&response.unwrap_err(), member);
    }
}

fn cross_input(tags: &[AwsSessionTag]) -> CrossAccountIrsa<'_> {
    CrossAccountIrsa {
        irsa_role_arn: IRSA_ROLE_ARN,
        role_arn: ROLE_ARN,
        session_name: SESSION_NAME,
        token_file: TOKEN_FILE,
        external_id: Some("external-1"),
        tags: Some(tags),
    }
}

fn cross_replies() -> BTreeMap<String, Answer> {
    replies([
        (
            "AssumeRoleWithWebIdentity",
            fixture::web_identity_ok(
                "ASIAIRSA",
                "irsa-secret",
                "irsa-session",
                "2026-01-01T00:30:00Z",
                None,
            ),
        ),
        (
            "AssumeRole",
            fixture::assume_role_ok(
                "ASIATARGET",
                "target-secret",
                "target-session",
                TARGET_EXPIRATION,
            ),
        ),
        ("GetCallerIdentity", fixture::access_denied()),
    ])
}

pub(super) fn cross_account_irsa() {
    let tags = tags();
    let input = cross_input(&tags);
    let (native, recorder) = fixture::sources(
        &ENVIRONMENT,
        &[(TOKEN_FILE, "  irsa-token\n")],
        cross_replies(),
    );
    let kwargs = fixture::kwargs(&ENVIRONMENT, Some(ENDPOINT), Some(REGION));
    let credentials = fixture::runtime()
        .block_on(handle_irsa_cross_account(&input, &kwargs, &native))
        .unwrap();
    assert_target(&credentials);
    assert_cross_calls(&calls(&recorder));
    assert_cross_failures(&input, &kwargs);
    assert_cross_missing_members(&input, &kwargs);
}

fn assert_cross_missing_members(input: &CrossAccountIrsa<'_>, kwargs: &StsClientKwargs) {
    for member in ["AccessKeyId", "SecretAccessKey", "SessionToken"] {
        let first_hop = cross_error(
            input,
            kwargs,
            &[(TOKEN_FILE, IRSA_TOKEN)],
            replies([(
                "AssumeRoleWithWebIdentity",
                fixture::web_identity_missing(member),
            )]),
        );
        fixture::assert_missing(&first_hop, member);
    }
    for member in [
        "AccessKeyId",
        "SecretAccessKey",
        "SessionToken",
        "Expiration",
    ] {
        let target = cross_error(
            input,
            kwargs,
            &[(TOKEN_FILE, IRSA_TOKEN)],
            replies([
                (
                    "AssumeRoleWithWebIdentity",
                    fixture::web_identity_ok("ASIAIRSA", "s", "t", "2026-01-01T00:30:00Z", None),
                ),
                ("AssumeRole", fixture::assume_role_missing(member)),
            ]),
        );
        fixture::assert_missing(&target, member);
    }
}

fn assert_cross_calls(exchanges: &[Exchange]) {
    assert_eq!(exchanges.len(), 3_usize);
    assert_irsa_assumption(&exchanges[0]);
    assert_identity_probe(&exchanges[1]);
    assert_target_assumption(&exchanges[2]);
}

fn assert_irsa_assumption(irsa: &Exchange) {
    assert_eq!(irsa.action, "AssumeRoleWithWebIdentity");
    assert_eq!(form(irsa, "RoleArn"), Some(IRSA_ROLE_ARN));
    assert_eq!(form(irsa, "RoleSessionName"), Some(SESSION_NAME));
    assert_eq!(form(irsa, "WebIdentityToken"), Some(IRSA_TOKEN));
    assert_eq!(form(irsa, "DurationSeconds"), None);
    assert_eq!(form(irsa, "Policy"), None);
    assert_eq!(form(irsa, "ExternalId"), None);
    assert!(irsa.form.keys().all(|name| !name.starts_with("Tags")));
    assert!(irsa.access_key.is_none());
}

fn assert_identity_probe(identity: &Exchange) {
    assert_eq!(identity.action, "GetCallerIdentity");
    assert_eq!(identity.access_key.as_deref(), Some("ASIAIRSA"));
    assert_eq!(identity.session_token.as_deref(), Some("irsa-session"));
}

fn assert_target_assumption(assume: &Exchange) {
    assert_eq!(assume.action, "AssumeRole");
    assert_eq!(form(assume, "RoleArn"), Some(ROLE_ARN));
    assert_eq!(form(assume, "ExternalId"), Some("external-1"));
    assert_eq!(tags_of(assume), expected_tags());
    assert_eq!(assume.access_key.as_deref(), Some("ASIAIRSA"));
    assert_eq!(assume.session_token.as_deref(), Some("irsa-session"));
}

fn cross_error(
    input: &CrossAccountIrsa<'_>,
    kwargs: &StsClientKwargs,
    files: &[(&str, &str)],
    answers: BTreeMap<String, Answer>,
) -> WebIdentityError {
    let (native, _recorder) = fixture::sources(&ENVIRONMENT, files, answers);
    fixture::runtime()
        .block_on(handle_irsa_cross_account(input, kwargs, &native))
        .unwrap_err()
}

fn assert_binary_token_file(input: &CrossAccountIrsa<'_>, kwargs: &StsClientKwargs) {
    let (binary, _binary_recorder) = fixture::sources_with_bytes(
        &ENVIRONMENT,
        &[(TOKEN_FILE, vec![b'i', b'r', 0xff])],
        BTreeMap::new(),
    );
    let error = fixture::runtime()
        .block_on(handle_irsa_cross_account(input, kwargs, &binary))
        .unwrap_err();
    assert!(matches!(error, WebIdentityError::TokenFileEncoding { .. }));
}

fn assert_cross_failures(input: &CrossAccountIrsa<'_>, kwargs: &StsClientKwargs) {
    let missing_file = cross_error(input, kwargs, &[], BTreeMap::new());
    assert!(matches!(
        missing_file,
        WebIdentityError::TokenFile { path, .. } if path.to_str() == Some(TOKEN_FILE)
    ));
    assert_binary_token_file(input, kwargs);
    let missing = cross_error(
        input,
        kwargs,
        &[(TOKEN_FILE, IRSA_TOKEN)],
        replies([(
            "AssumeRoleWithWebIdentity",
            fixture::web_identity_without_credentials(None),
        )]),
    );
    assert!(matches!(
        missing,
        WebIdentityError::MissingField(message) if message.ends_with("no Credentials")
    ));
    let denied = cross_error(
        input,
        kwargs,
        &[(TOKEN_FILE, IRSA_TOKEN)],
        replies([(
            "AssumeRoleWithWebIdentity",
            fixture::web_identity_ok("ASIAIRSA", "s", "t", "2026-01-01T00:30:00Z", None),
        )]),
    );
    assert!(matches!(denied, WebIdentityError::AssumeRole { .. }));
}

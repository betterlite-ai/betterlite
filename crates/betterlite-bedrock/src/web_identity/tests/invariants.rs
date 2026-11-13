use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::ExposeSecret as _;

use super::fixture::{self, Exchange};
use crate::aws_endpoints::sts::StsClientKwargs;
use crate::web_identity::auth::{
    WebIdentityAuth, WebIdentityCredentials, auth_with_web_identity_token,
};
use crate::web_identity::error::WebIdentityError;

const ENVIRONMENT: [(&str, &str); 4] = [
    ("AWS_ACCESS_KEY_ID", "AKIAAMBIENT"),
    ("AWS_SECRET_ACCESS_KEY", "ambient-secret"),
    ("AWS_SESSION_TOKEN", "ambient-session"),
    ("AWS_REGION", "us-east-1"),
];
const ENDPOINT: &str = "https://sts.us-east-1.amazonaws.com";
const REGION: &str = "us-east-1";
const EXPIRATION: &str = "2026-01-01T01:00:00Z";

struct Identity {
    reference: &'static str,
    token: &'static str,
    role: &'static str,
    session: &'static str,
    access_key: &'static str,
    secret_key: &'static str,
    session_token: &'static str,
}

fn run(
    identity: &Identity,
) -> (
    Result<WebIdentityCredentials, WebIdentityError>,
    Vec<Exchange>,
) {
    let answer = fixture::web_identity_ok(
        identity.access_key,
        identity.secret_key,
        identity.session_token,
        EXPIRATION,
        Some(1_i32),
    );
    let (native, recorder) = fixture::sources(
        &ENVIRONMENT,
        &[],
        BTreeMap::from([("AssumeRoleWithWebIdentity".to_owned(), answer)]),
    );
    let secrets = fixture::secrets(Some(identity.token));
    let kwargs = fixture::kwargs(&ENVIRONMENT, Some(ENDPOINT), Some(REGION));
    let auth = WebIdentityAuth {
        token_reference: identity.reference,
        role_arn: identity.role,
        session_name: identity.session,
        external_id: None,
        max_policy_size: 75_i64,
    };
    let result = fixture::runtime().block_on(auth_with_web_identity_token(
        &auth, &kwargs, &secrets, &native,
    ));
    let exchanges = recorder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (result, exchanges)
}

fn form<'exchange>(exchange: &'exchange Exchange, name: &str) -> Option<&'exchange str> {
    exchange.form.get(name).map(String::as_str)
}

fn identity(first: bool) -> Identity {
    if first {
        Identity {
            reference: "identity-one-token",
            token: "token-one-value",
            role: "arn:aws:iam::123456789012:role/FirstRole",
            session: "first-session",
            access_key: "ASIAFIRST",
            secret_key: "first-secret",
            session_token: "first-token",
        }
    } else {
        Identity {
            reference: "identity-two-token",
            token: "token-two-value",
            role: "arn:aws:iam::123456789012:role/SecondRole",
            session: "second-session",
            access_key: "ASIASECOND",
            secret_key: "second-secret",
            session_token: "second-token",
        }
    }
}

#[test]
fn bedrock_web_identity_credential_identity_isolation() {
    let first = identity(true);
    let second = identity(false);
    let (first_result, first_calls) = run(&first);
    let (second_result, second_calls) = run(&second);
    let first_credentials = first_result.unwrap();
    let second_credentials = second_result.unwrap();
    assert_eq!(first_calls.len(), 1_usize);
    assert_eq!(second_calls.len(), 1_usize);
    assert_call(&first_calls[0], &first);
    assert_call(&second_calls[0], &second);
    assert_ne!(
        form(&second_calls[0], "WebIdentityToken"),
        Some(first.token)
    );
    assert_eq!(
        first_credentials.credentials.access_key_id.expose_secret(),
        first.access_key
    );
    assert_eq!(
        second_credentials.credentials.access_key_id.expose_secret(),
        second.access_key
    );
    assert_eq!(
        first_credentials
            .credentials
            .secret_access_key
            .expose_secret(),
        first.secret_key
    );
    assert_eq!(
        second_credentials
            .credentials
            .secret_access_key
            .expose_secret(),
        second.secret_key
    );
    assert_ne!(
        first_credentials.credentials.access_key_id.expose_secret(),
        second_credentials.credentials.access_key_id.expose_secret()
    );
}

fn assert_call(exchange: &Exchange, identity: &Identity) {
    assert_eq!(form(exchange, "WebIdentityToken"), Some(identity.token));
    assert_eq!(form(exchange, "RoleArn"), Some(identity.role));
    assert_eq!(form(exchange, "RoleSessionName"), Some(identity.session));
}

fn auth_for_failure() -> WebIdentityAuth<'static> {
    WebIdentityAuth {
        token_reference: "web-identity-token",
        role_arn: "arn:aws:iam::123456789012:role/TargetRole",
        session_name: "bedrock-session",
        external_id: None,
        max_policy_size: 75_i64,
    }
}

#[test]
fn bedrock_web_identity_secret_safe_failure() {
    const SECRET: &str = "SECRET-FIXTURE-TOKEN";
    let (native, recorder) = fixture::sources(
        &ENVIRONMENT,
        &[],
        BTreeMap::from([(
            "AssumeRoleWithWebIdentity".to_owned(),
            fixture::invalid_identity_token(),
        )]),
    );
    let secrets = fixture::secrets(Some(SECRET));
    let kwargs = fixture::kwargs(&ENVIRONMENT, Some(ENDPOINT), Some(REGION));
    let auth = auth_for_failure();
    let error = fixture::runtime()
        .block_on(auth_with_web_identity_token(
            &auth, &kwargs, &secrets, &native,
        ))
        .unwrap_err();
    let calls = recorder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(form(&calls[0], "WebIdentityToken"), Some(SECRET));
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert!(!display.contains(SECRET), "{display}");
    assert!(!debug.contains(SECRET), "{debug}");
    assert_success_debug_is_redacted(&auth, &kwargs, &secrets, SECRET);
}

fn assert_success_debug_is_redacted(
    auth: &WebIdentityAuth<'_>,
    kwargs: &StsClientKwargs,
    secrets: &SecretLookup,
    secret: &str,
) {
    let answer = fixture::web_identity_ok(
        "ASIASECRET",
        "SECRET-ACCESS",
        "SECRET-SESSION",
        EXPIRATION,
        Some(1_i32),
    );
    let (native, _recorder) = fixture::sources(
        &ENVIRONMENT,
        &[],
        BTreeMap::from([("AssumeRoleWithWebIdentity".to_owned(), answer)]),
    );
    let credentials = fixture::runtime()
        .block_on(auth_with_web_identity_token(auth, kwargs, secrets, &native))
        .unwrap();
    let debug = format!("{credentials:?}");
    assert!(!debug.contains("SECRET-ACCESS"), "{debug}");
    assert!(!debug.contains("SECRET-SESSION"), "{debug}");
    assert!(!debug.contains(secret), "{debug}");
}

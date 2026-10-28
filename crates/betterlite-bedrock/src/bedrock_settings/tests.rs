#![cfg(test)]

mod errors;
mod settings;

use errors::{bedrock_error_response, error_text_unread, input, text};
use settings::{deployment_before_env, empty_bearer, exposed, irsa_config, load, session_tags};

use super::{BedrockError, ErrorHeaders};

#[rstest::rstest]
#[case::deployment_before_env(deployment_before_env)]
#[case::empty_bearer(empty_bearer)]
#[case::session_tags(session_tags)]
#[case::irsa_config(irsa_config)]
#[case::bedrock_error_response(bedrock_error_response)]
#[case::error_text_unread(error_text_unread)]
fn bedrock_settings_cases(#[case] case: fn()) {
    case();
}

#[test]
fn bedrock_settings_credential_identity_isolation() {
    let settings = load(
        &[
            ", aws_access_key_id: first-key, aws_secret_access_key: first-secret, aws_role_name: first-role",
            ", aws_access_key_id: second-key, aws_secret_access_key: second-secret",
            "",
        ],
        &[("AWS_SECRET_ACCESS_KEY", "env-secret")],
    )
    .unwrap();
    let identities = settings
        .iter()
        .map(|aws| {
            (
                exposed(aws.access_key_id.as_ref()),
                exposed(aws.secret_access_key.as_ref()),
                aws.role_name.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        identities,
        [
            (Some("first-key"), Some("first-secret"), Some("first-role")),
            (Some("second-key"), Some("second-secret"), None),
            (None, Some("env-secret"), None),
        ]
    );
}

#[test]
fn bedrock_settings_secret_safe_failure() {
    let environment = [
        ("AWS_SECRET_ACCESS_KEY", "fixture-env-secret"),
        ("AWS_BEARER_TOKEN_BEDROCK", "fixture-bearer"),
        ("AWS_SESSION_TOKEN", "fixture-session"),
    ];
    let configured = ", aws_access_key_id: fixture-key, aws_web_identity_token: fixture-web";
    let debug = format!("{:?}", load(&[configured], &environment).unwrap());
    let missing = ", aws_secret_access_key: os.environ/MISSING_SECRET";
    let failure = load(&[missing], &environment).unwrap_err();
    let secret_header = ("authorization".to_owned(), text("fixture-s\u{e9}cret"));
    let headers = Some(ErrorHeaders::Mapping(vec![secret_header]));
    let header = BedrockError::try_from(input(headers, None)).unwrap_err();
    let rendered = format!("{debug} {failure} {failure:?} {header} {header:?}");
    let secrets = [
        "fixture-env-secret",
        "fixture-bearer",
        "fixture-session",
        "fixture-key",
        "fixture-web",
        "s\u{e9}cret",
    ];
    let leaked: Vec<&str> = secrets
        .into_iter()
        .filter(|secret| rendered.contains(secret))
        .collect();
    assert!(leaked.is_empty(), "{leaked:?} leaked");
    assert!(rendered.contains("MISSING_SECRET"));
}

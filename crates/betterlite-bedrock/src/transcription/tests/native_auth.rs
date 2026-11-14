use std::collections::BTreeMap;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::{env, params};
use crate::aws_settings::sources::Credentials;
use crate::transcription::auth::{
    CredentialSource, SECRET_NAMES, credential_source, validate_environment,
};

const CHAIN_NAMES: [&str; 10] = [
    "ACCESS_KEY_ID",
    "SECRET_ACCESS_KEY",
    "SESSION_TOKEN",
    "REGION_NAME",
    "SESSION_NAME",
    "PROFILE_NAME",
    "ROLE_NAME",
    "WEB_IDENTITY_TOKEN",
    "STS_ENDPOINT",
    "EXTERNAL_ID",
];

const AMBIENT: [(&str, &str); 2] = [("AWS_ACCESS_KEY_ID", "ambient"), ("AWS_ROLE_NAME", "role")];

const PARTIAL: [&str; 4] = [
    r#"{"aws_access_key_id":"AK"}"#,
    r#"{"aws_secret_access_key":"SK"}"#,
    r#"{"aws_access_key_id":"  ","aws_secret_access_key":"SK"}"#,
    r#"{"aws_access_key_id":"AK","aws_secret_access_key":""}"#,
];

const OVERRIDES: &str = r#"{"aws_access_key_id":"p1","aws_session_token":"p3","aws_region_name":"p4","aws_session_name":"p5","aws_profile_name":"p6","aws_role_name":"p7","aws_web_identity_token":"p8","aws_sts_endpoint":"p9","aws_external_id":"p10"}"#;

fn pair(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

fn host_credentials(source: &str) -> Credentials {
    match credential_source(&params(source), &env(&AMBIENT)) {
        CredentialSource::HostSupplied(credentials) => *credentials,
        CredentialSource::Chain(_) => panic!("host supplied credentials expected: {source}"),
    }
}

fn chain_settings(source: &str, variables: &BTreeMap<String, String>) -> AwsSettings {
    match credential_source(&params(source), variables) {
        CredentialSource::Chain(settings) => *settings,
        CredentialSource::HostSupplied(_) => panic!("chain expected: {source}"),
    }
}

fn exposed(value: Option<&SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

fn chain_fields(settings: &AwsSettings) -> [Option<String>; 10] {
    [
        exposed(settings.access_key_id.as_ref()),
        exposed(settings.secret_access_key.as_ref()),
        exposed(settings.session_token.as_ref()),
        settings.region_name.clone(),
        settings.session_name.clone(),
        settings.profile_name.clone(),
        settings.role_name.clone(),
        exposed(settings.web_identity_token.as_ref()),
        settings.sts_endpoint.clone(),
        settings.external_id.clone(),
    ]
}

fn headers_and_region() {
    let none = params("{}");
    let plain = validate_environment(vec![], "bedrock/us-east-1/m", &none, &env(&[]));
    assert_eq!(plain.headers, [pair("Content-Type", "application/json")]);
    assert_eq!(plain.region, "us-east-1");
    let supplied = vec![pair("content-type", "text/plain"), pair("X-Extra", "1")];
    let variables = env(&[("AWS_REGION", "r2")]);
    let kept = validate_environment(supplied.clone(), "m", &none, &variables);
    assert_eq!(kept.headers, supplied);
    assert_eq!(kept.region, "r2");
    assert_eq!(SECRET_NAMES.len(), 11);
    assert_eq!(
        SECRET_NAMES[..3],
        [
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN"
        ]
    );
    assert_eq!(SECRET_NAMES[10], "AWS_EXTERNAL_ID");
}

fn host_supplied() {
    let hosted = r#"{"aws_access_key_id":" AK ","aws_secret_access_key":" SK ","aws_session_token":" ST ","aws_role_name":"x"}"#;
    let credentials = host_credentials(hosted);
    assert_eq!(credentials.access_key_id.expose_secret(), "AK");
    assert_eq!(credentials.secret_access_key.expose_secret(), "SK");
    assert_eq!(
        exposed(credentials.session_token.as_ref()).as_deref(),
        Some("ST")
    );
    let blank =
        r#"{"aws_access_key_id":"AK","aws_secret_access_key":"SK","aws_session_token":"  "}"#;
    assert!(host_credentials(blank).session_token.is_none());
    for partial in PARTIAL {
        let source = credential_source(&params(partial), &env(&AMBIENT));
        assert!(matches!(source, CredentialSource::Chain(_)), "{partial}");
    }
}

fn chain_from_environment() {
    let variables: BTreeMap<String, String> = CHAIN_NAMES
        .iter()
        .map(|name| (format!("AWS_{name}"), format!("env-{name}")))
        .collect();
    let from_env = chain_fields(&chain_settings("{}", &variables));
    assert_eq!(
        from_env,
        CHAIN_NAMES.map(|name| Some(format!("env-{name}")))
    );
    let overridden = chain_fields(&chain_settings(OVERRIDES, &variables));
    let expected = [
        "p1",
        "env-SECRET_ACCESS_KEY",
        "p3",
        "p4",
        "p5",
        "p6",
        "p7",
        "p8",
        "p9",
        "p10",
    ];
    assert_eq!(overridden, expected.map(|value| Some(value.to_owned())));
    let empty = chain_settings("{}", &BTreeMap::new());
    assert!(chain_fields(&empty).iter().all(Option::is_none));
}

pub(super) fn native_transcription_auth() {
    headers_and_region();
    host_supplied();
    chain_from_environment();
}

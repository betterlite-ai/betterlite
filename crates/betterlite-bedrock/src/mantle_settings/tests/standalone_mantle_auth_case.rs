#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "one scenario function per oracle case reads top to bottom as its fixture steps"
    )
)]

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_settings::auth::{SignRequest, sign_request};
use crate::mantle_settings::error::MantleSettingsError;
use crate::signing::error::SigningError;
use crate::signing::request::AwsSignedRequest;

use super::support;

const API_BASE: &str = "https://bedrock-mantle.us-east-1.api.aws/v1";
const MANTLE_BASE: &str = "https://bedrock-mantle.eu-west-3.api.aws/openai/v1";

fn data() -> IndexMap<String, Json> {
    IndexMap::from([(
        String::from("messages"),
        Json::List(vec![Json::Object(IndexMap::new())]),
    )])
}

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn header<'signed>(signed: &'signed AwsSignedRequest, name: &str) -> Option<&'signed str> {
    signed
        .headers
        .iter()
        .find(|(stored, _)| stored.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn caller_headers() -> Vec<(String, Option<String>)> {
    vec![
        (String::from("X-Caller"), Some(String::from("fixture"))),
        (
            String::from("Authorization"),
            Some(String::from("Bearer caller-fixture-key")),
        ),
        (
            String::from("authorization"),
            Some(String::from("Bearer caller-lower-key")),
        ),
    ]
}

fn globals(pairs: &[(&str, &str)]) -> MantleGlobals {
    support::globals(
        &pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<String, String>>(),
    )
}

pub(super) fn standalone_mantle_auth() {
    let resolved = |_: &AwsSettings, _: &str| async {
        Ok::<Credentials, CredentialError>(support::fixture_credentials())
    };
    let empty = globals(&[]);
    let data = data();
    let headers = caller_headers();
    let params = support::object("{}");
    let explicit = support::secret("explicit-fixture-key");
    let bearer = support::block_on(sign_request(
        SignRequest {
            headers: &headers,
            optional_params: &params,
            request_data: &data,
            api_base: API_BASE,
            api_key: Some(&explicit),
            model: Some("openai.gpt-oss-120b"),
            stream: Some(true),
            fake_stream: Some(false),
            globals: &empty,
            now: now(),
        },
        resolved,
    ))
    .expect("the bearer request signs");
    assert_eq!(
        header(&bearer, "Authorization"),
        Some("Bearer explicit-fixture-key")
    );
    assert_eq!(header(&bearer, "Content-Type"), Some("application/json"));
    assert_eq!(header(&bearer, "X-Caller"), Some("fixture"));
    assert_eq!(bearer.url, API_BASE);
    assert_eq!(
        serde_json::from_slice::<betterlite_openai::openai_base_config::Object>(&bearer.body)
            .expect("the body is JSON"),
        support::object(r#"{"messages": [{}]}"#)
    );
    // An empty explicit key falls through to the env chain.
    let env_globals = globals(&[("BEDROCK_MANTLE_API_KEY", "mantle-fixture-key")]);
    let empty_key = support::secret("");
    let from_env = support::block_on(sign_request(
        SignRequest {
            headers: &[],
            optional_params: &params,
            request_data: &data,
            api_base: API_BASE,
            api_key: Some(&empty_key),
            model: None,
            stream: None,
            fake_stream: None,
            globals: &env_globals,
            now: now(),
        },
        resolved,
    ))
    .expect("the env bearer request signs");
    assert_eq!(
        header(&from_env, "Authorization"),
        Some("Bearer mantle-fixture-key")
    );
    // Without a bearer the host region pins the scope even against a conflicting region parameter, and both caller Authorization spellings are dropped.
    let conflicting = support::object(r#"{"aws_region_name": "us-east-2"}"#);
    let sigv4 = support::block_on(sign_request(
        SignRequest {
            headers: &headers,
            optional_params: &conflicting,
            request_data: &data,
            api_base: MANTLE_BASE,
            api_key: None,
            model: Some("openai.gpt-oss-120b"),
            stream: None,
            fake_stream: None,
            globals: &empty,
            now: now(),
        },
        resolved,
    ))
    .expect("the SigV4 request signs");
    assert_eq!(sigv4.url, MANTLE_BASE);
    assert_eq!(header(&sigv4, "X-Caller"), Some("fixture"));
    let scope = header(&sigv4, "Authorization").expect("the signature carries Authorization");
    assert!(scope.starts_with(
        "AWS4-HMAC-SHA256 Credential=fixture-access-key/20231114/eu-west-3/bedrock/aws4_request"
    ));
    assert!(header(&sigv4, "X-Amz-Date").is_some());
    // A custom base keeps the explicit region.
    let custom_region = support::object(r#"{"aws_region_name": "us-east-2"}"#);
    let custom = support::block_on(sign_request(
        SignRequest {
            headers: &[],
            optional_params: &custom_region,
            request_data: &data,
            api_base: "https://proxy.example/v1",
            api_key: None,
            model: None,
            stream: None,
            fake_stream: None,
            globals: &empty,
            now: now(),
        },
        resolved,
    ))
    .expect("the custom base signs");
    let custom_scope =
        header(&custom, "Authorization").expect("the signature carries Authorization");
    assert!(custom_scope.contains("/20231114/us-east-2/bedrock/aws4_request"));
    // Caller AWS auth parameters travel in optional_params and reach the credential resolver.
    let auth_params = support::object(
        r#"{"aws_access_key_id": "caller-access-key", "aws_secret_access_key": "caller-secret-key", "aws_region_name": "us-east-2"}"#,
    );
    let observed_auth = |settings: &AwsSettings, _region: &str| {
        assert_eq!(
            settings
                .access_key_id
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            Some(String::from("caller-access-key"))
        );
        assert_eq!(
            settings
                .secret_access_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            Some(String::from("caller-secret-key"))
        );
        async { Ok::<Credentials, CredentialError>(support::fixture_credentials()) }
    };
    let keyed = support::block_on(sign_request(
        SignRequest {
            headers: &[],
            optional_params: &auth_params,
            request_data: &data,
            api_base: "https://proxy.example/v1",
            api_key: None,
            model: None,
            stream: None,
            fake_stream: None,
            globals: &empty,
            now: now(),
        },
        observed_auth,
    ))
    .expect("the caller auth parameters sign");
    assert!(
        header(&keyed, "Authorization")
            .expect("the signature carries Authorization")
            .contains("/20231114/us-east-2/bedrock/aws4_request")
    );
    // With no region anywhere the shared chain falls back to us-east-1.
    let fallback = support::block_on(sign_request(
        SignRequest {
            headers: &[],
            optional_params: &params,
            request_data: &data,
            api_base: "https://proxy.example/v1",
            api_key: None,
            model: None,
            stream: None,
            fake_stream: None,
            globals: &empty,
            now: now(),
        },
        resolved,
    ))
    .expect("the fallback region signs");
    let fallback_scope =
        header(&fallback, "Authorization").expect("the signature carries Authorization");
    assert!(fallback_scope.contains("/20231114/us-east-1/bedrock/aws4_request"));
    // Region-selection failures happen before the signer and pass through.
    let invalid = support::object(r#"{"aws_region_name": "NOT A REGION"}"#);
    let rejected = support::block_on(sign_request(
        SignRequest {
            headers: &[],
            optional_params: &invalid,
            request_data: &data,
            api_base: "https://proxy.example/v1",
            api_key: None,
            model: None,
            stream: None,
            fake_stream: None,
            globals: &empty,
            now: now(),
        },
        resolved,
    ))
    .expect_err("the invalid region fails");
    assert!(matches!(rejected, MantleSettingsError::Region(_)));
    // A host whose captured region is not a valid region still takes precedence, and the signer rejects it rather than the parameter.
    let upper = support::block_on(sign_request(
        SignRequest {
            headers: &[],
            optional_params: &custom_region,
            request_data: &data,
            api_base: "https://bedrock-mantle.US-EAST-1.api.aws",
            api_key: None,
            model: None,
            stream: None,
            fake_stream: None,
            globals: &empty,
            now: now(),
        },
        resolved,
    ))
    .expect_err("the uppercase host region fails in the signer");
    assert!(matches!(
        upper,
        MantleSettingsError::Signing(SigningError::Endpoint(_))
    ));
}

#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "one scenario function per oracle case reads top to bottom as its fixture steps"
    )
)]

use std::time::{Duration, SystemTime};

use aws_credential_types::provider::error::CredentialsError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::aws_settings::AuthError;
use crate::aws_settings::sources::{Credentials, SourceError};
use crate::credentials::error::CredentialError;
use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_settings::auth::{SignRequest, sign_request};
use crate::mantle_settings::error::{AUTH_FAILURE_MESSAGE, MantleSettingsError};
use crate::signing::error::SigningError;

use super::support;

const LITELLM_MESSAGE: &str = "Bedrock Mantle auth failed: no Bearer token and no usable AWS credentials. Set BEDROCK_MANTLE_API_KEY (or AWS_BEARER_TOKEN_BEDROCK) or pass api_key for Bearer auth, or provide AWS credentials (IAM role / access key / profile / web identity) for SigV4.";

fn data() -> IndexMap<String, Json> {
    IndexMap::from([(
        String::from("messages"),
        Json::List(vec![Json::Object(IndexMap::new())]),
    )])
}

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn request<'text>(
    params: &'text betterlite_openai::openai_base_config::Object,
    globals: &'text MantleGlobals,
    api_base: &'text str,
    data: &'text IndexMap<String, Json>,
) -> SignRequest<'text> {
    SignRequest {
        headers: &[],
        optional_params: params,
        request_data: data,
        api_base,
        api_key: None,
        model: Some("openai.gpt-oss-120b"),
        stream: None,
        fake_stream: None,
        globals,
        now: now(),
    }
}

pub(super) fn mantle_auth_failure() {
    let missing_provider = |_: &AwsSettings, _: &str| async {
        Err::<Credentials, CredentialError>(CredentialError::Source(SourceError::MissingProvider))
    };
    let no_credentials = |_: &AwsSettings, _: &str| async {
        Err::<Credentials, CredentialError>(CredentialError::MissingCredentials)
    };
    let partial_credentials = |_: &AwsSettings, _: &str| async {
        Err::<Credentials, CredentialError>(CredentialError::Source(SourceError::Provider(
            CredentialsError::provider_error("fixture-partial-credentials"),
        )))
    };
    let unexpected_auth = |_: &AwsSettings, _: &str| async {
        Err::<Credentials, CredentialError>(CredentialError::Auth(AuthError::SessionTags))
    };
    let unused = |_: &AwsSettings, _: &str| async {
        Err::<Credentials, CredentialError>(panic!(
            "the signer must not look up credentials for a request it rejects first"
        ))
    };
    assert_eq!(AUTH_FAILURE_MESSAGE, LITELLM_MESSAGE);
    let globals = support::empty_globals();
    let data = data();
    let params = support::object("{}");
    let api_base = "https://bedrock-mantle.us-east-1.api.aws/v1";
    // The four botocore lookup failures LiteLLM catches all land on the same message.
    let missing = support::block_on(sign_request(
        request(&params, &globals, api_base, &data),
        missing_provider,
    ))
    .expect_err("a missing provider fails the request");
    assert!(matches!(missing, MantleSettingsError::Auth { .. }));
    assert_eq!(missing.to_string(), LITELLM_MESSAGE);
    let cause = std::error::Error::source(&missing).expect("the cause is preserved");
    assert_eq!(cause.to_string(), "the AWS credential lookup failed");
    assert_eq!(
        cause.source().map(ToString::to_string).as_deref(),
        Some("AWS credential provider is unavailable")
    );
    let empty = support::block_on(sign_request(
        request(&params, &globals, api_base, &data),
        no_credentials,
    ))
    .expect_err("missing credentials fail the request");
    assert!(matches!(empty, MantleSettingsError::Auth { .. }));
    assert_eq!(empty.to_string(), LITELLM_MESSAGE);
    let partial = support::block_on(sign_request(
        request(&params, &globals, api_base, &data),
        partial_credentials,
    ))
    .expect_err("partial credentials fail the request");
    assert!(matches!(partial, MantleSettingsError::Auth { .. }));
    assert_eq!(partial.to_string(), LITELLM_MESSAGE);
    assert!(std::error::Error::source(&partial).is_some());
    // The provider's text stays out of the surfaced error and its Debug.
    assert!(!format!("{partial:?}").contains("fixture-partial-credentials"));
    // A lookup failure outside the caught classes passes through unchanged.
    let other = support::block_on(sign_request(
        request(&params, &globals, api_base, &data),
        unexpected_auth,
    ))
    .expect_err("an unexpected auth failure passes through");
    assert!(matches!(
        other,
        MantleSettingsError::Signing(SigningError::Credential(CredentialError::Auth(
            AuthError::SessionTags
        )))
    ));
    assert_ne!(other.to_string(), LITELLM_MESSAGE);
    // Region selection fails before the signer runs, so its own error surfaces and no credentials are read.
    let invalid = support::object(r#"{"aws_region_name": "EU-WEST-1"}"#);
    let region = support::block_on(sign_request(
        request(&invalid, &globals, "https://proxy.example/v1", &data),
        unused,
    ))
    .expect_err("the invalid region fails before signing");
    assert!(matches!(region, MantleSettingsError::Region(_)));
    assert_ne!(region.to_string(), LITELLM_MESSAGE);
    // Signer failures that are neither credential lookups nor region selection pass through too.
    let uppercase_host = support::block_on(sign_request(
        request(
            &params,
            &globals,
            "https://bedrock-mantle.US-EAST-1.api.aws",
            &data,
        ),
        unused,
    ))
    .expect_err("the uppercase host region fails inside the signer");
    assert!(matches!(
        uppercase_host,
        MantleSettingsError::Signing(SigningError::Endpoint(_))
    ));
    assert_ne!(uppercase_host.to_string(), LITELLM_MESSAGE);
}

#![cfg(test)]

mod auth_failure_case;
mod chat_provider_prefix_case;
mod mantle_passthrough_path_case;
mod mantle_passthrough_reply_shape_case;
mod messages_auth_and_url_case;
mod standalone_mantle_auth_case;
mod standalone_mantle_base_case;
mod support;

use rstest::rstest;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    StandaloneMantleBase,
    StandaloneMantleAuth,
    ChatProviderPrefix,
    MessagesAuthAndUrl,
    MantlePassthroughPath,
    MantleAuthFailure,
    MantlePassthroughReplyShape,
}

// One row per bedrock-mantle-settings oracle case file.
#[rstest]
#[case::standalone_mantle_base(Scenario::StandaloneMantleBase)]
#[case::standalone_mantle_auth(Scenario::StandaloneMantleAuth)]
#[case::chat_provider_prefix(Scenario::ChatProviderPrefix)]
#[case::messages_auth_and_url(Scenario::MessagesAuthAndUrl)]
#[case::mantle_passthrough_path(Scenario::MantlePassthroughPath)]
#[case::mantle_auth_failure(Scenario::MantleAuthFailure)]
#[case::mantle_passthrough_reply_shape(Scenario::MantlePassthroughReplyShape)]
fn mantle_settings_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::StandaloneMantleBase => standalone_mantle_base_case::standalone_mantle_base(),
        Scenario::StandaloneMantleAuth => standalone_mantle_auth_case::standalone_mantle_auth(),
        Scenario::ChatProviderPrefix => chat_provider_prefix_case::chat_provider_prefix(),
        Scenario::MessagesAuthAndUrl => messages_auth_and_url_case::messages_auth_and_url(),
        Scenario::MantlePassthroughPath => mantle_passthrough_path_case::mantle_passthrough_path(),
        Scenario::MantleAuthFailure => auth_failure_case::mantle_auth_failure(),
        Scenario::MantlePassthroughReplyShape => {
            mantle_passthrough_reply_shape_case::mantle_passthrough_reply_shape()
        }
    }
}

#[test]
fn bedrock_mantle_settings_credential_identity_isolation() {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use betterlite_config::sections::bedrock_settings::AwsSettings;
    use betterlite_models::decisions::Json;
    use indexmap::IndexMap;

    use crate::aws_settings::sources::Credentials;
    use crate::credentials::error::CredentialError;
    use crate::mantle_settings::auth::{SignRequest, sign_request};
    use crate::mantle_settings::chat::MantleChatConfig;

    async fn first(_settings: &AwsSettings, _region: &str) -> Result<Credentials, CredentialError> {
        Ok(support::credentials("first-access-key", "first-secret-key"))
    }
    async fn second(
        _settings: &AwsSettings,
        _region: &str,
    ) -> Result<Credentials, CredentialError> {
        Ok(support::credentials(
            "second-access-key",
            "second-secret-key",
        ))
    }

    let first_cache = support::credential_cache();
    let second_cache = support::credential_cache();
    assert!(!Arc::ptr_eq(&first_cache, &second_cache));
    let first_config = MantleChatConfig::new(Some(Arc::clone(&first_cache)));
    let second_config = MantleChatConfig::new(Some(Arc::clone(&second_cache)));
    assert!(
        first_config
            .credential_cache
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &first_cache))
    );
    assert!(
        second_config
            .credential_cache
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &second_cache))
    );
    assert!(!format!("{first_config:?}").contains("second"));
    let data = IndexMap::from([(String::from("model"), Json::Text(String::from("fixture")))]);
    let globals = support::empty_globals();
    let params = support::object("{}");
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let request = SignRequest {
        headers: &[],
        optional_params: &params,
        request_data: &data,
        api_base: "https://bedrock-mantle.us-east-1.api.aws/v1",
        api_key: None,
        model: Some("openai.gpt-oss-120b"),
        stream: None,
        fake_stream: None,
        globals: &globals,
        now,
    };
    let first_signed =
        support::block_on(sign_request(request, first)).expect("the first identity signs");
    let second_signed =
        support::block_on(sign_request(request, second)).expect("the second identity signs");
    let authorization = |signed: &crate::signing::request::AwsSignedRequest| {
        signed
            .headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value.clone())
            .expect("the signature carries Authorization")
    };
    let first_authorization = authorization(&first_signed);
    let second_authorization = authorization(&second_signed);
    assert!(
        first_authorization.contains("Credential=first-access-key/20231114/us-east-1/bedrock/")
    );
    assert!(
        second_authorization.contains("Credential=second-access-key/20231114/us-east-1/bedrock/")
    );
    assert!(!first_authorization.contains("second-access-key"));
    assert!(!second_authorization.contains("first-access-key"));
    assert!(!first_authorization.contains("second-secret-key"));
    assert!(!format!("{first_signed:?}").contains("first-secret-key"));
    assert!(!format!("{second_signed:?}").contains("second-secret-key"));
}

#[test]
fn bedrock_mantle_settings_secret_safe_failure() {
    use std::time::{Duration, SystemTime};

    use aws_credential_types::provider::error::CredentialsError;
    use betterlite_config::sections::bedrock_settings::AwsSettings;
    use betterlite_models::decisions::Json;
    use indexmap::IndexMap;
    use secrecy::ExposeSecret as _;

    use crate::aws_settings::sources::{Credentials, SourceError};
    use crate::credentials::error::CredentialError;
    use crate::mantle_settings::auth::{SignRequest, sign_request};
    use crate::mantle_settings::error::{AUTH_FAILURE_MESSAGE, MantleSettingsError};

    async fn leaked(
        _settings: &AwsSettings,
        _region: &str,
    ) -> Result<Credentials, CredentialError> {
        Err(CredentialError::Source(SourceError::Provider(
            CredentialsError::provider_error("fixture-leaked-secret"),
        )))
    }

    let data = IndexMap::from([(String::from("model"), Json::Text(String::from("fixture")))]);
    let globals = support::empty_globals();
    let params = support::object("{}");
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let request = SignRequest {
        headers: &[],
        optional_params: &params,
        request_data: &data,
        api_base: "https://bedrock-mantle.us-east-1.api.aws/v1",
        api_key: None,
        model: Some("openai.gpt-oss-120b"),
        stream: None,
        fake_stream: None,
        globals: &globals,
        now,
    };
    let error = support::block_on(sign_request(request, leaked))
        .expect_err("a credential-source failure fails the request");
    assert!(matches!(error, MantleSettingsError::Auth { .. }));
    assert_eq!(error.to_string(), AUTH_FAILURE_MESSAGE);
    assert!(std::error::Error::source(&error).is_some());
    assert!(!error.to_string().contains("fixture-leaked-secret"));
    assert!(!format!("{error:?}").contains("fixture-leaked-secret"));
    let bearer_params = support::object("{}");
    let bearer = support::secret("fixture-bearer-secret");
    let signed = support::block_on(sign_request(
        SignRequest {
            headers: &[],
            optional_params: &bearer_params,
            request_data: &data,
            api_base: "https://bedrock-mantle.us-east-1.api.aws/v1",
            api_key: Some(&bearer),
            model: None,
            stream: None,
            fake_stream: None,
            globals: &globals,
            now,
        },
        leaked,
    ))
    .expect("the bearer path signs");
    assert!(!format!("{signed:?}").contains(bearer.expose_secret()));
}

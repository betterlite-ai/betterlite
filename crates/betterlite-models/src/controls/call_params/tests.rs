#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::controls::call_params::{
    RequestCall, TRUSTED_CREDENTIALS_KEY, TrustedCall, TrustedCredentials, call_params,
    with_control_options,
};
use crate::controls::control_options::ControlOptions;

enum Scenario {
    RejectsClientField {
        request: &'static str,
        field: &'static str,
    },
    AcceptsRequestIdentity,
    MetadataCannotReplaceTrustedFields,
    KeepsTrustedDestination,
    StoresNonDefaultControl,
    DropsDefaultControl,
    CarriesTrustedCredentials,
}

fn trusted_call() -> TrustedCall {
    TrustedCall {
        model: Some("azure/gpt-4o".to_owned()),
        api_key: Some(SecretString::from("server-key")),
        api_base: Some("https://configured.test".to_owned()),
        custom_llm_provider: Some("azure".to_owned()),
        provider_affinity_header: Some("x-session".to_owned()),
        control: ControlOptions::new(Some(512)).expect("a valid control"),
        credentials: None,
    }
}

#[rstest]
#[case::client_api_key(
    Scenario::RejectsClientField { request: r#"{"api_key":"client-key"}"#, field: "api_key" }
)]
#[case::client_api_base(
    Scenario::RejectsClientField { request: r#"{"api_base":"https://client.test"}"#, field: "api_base" }
)]
#[case::client_provider(
    Scenario::RejectsClientField { request: r#"{"custom_llm_provider":"openai"}"#, field: "custom_llm_provider" }
)]
#[case::client_affinity_header(
    Scenario::RejectsClientField { request: r#"{"provider_affinity_header":"x-client"}"#, field: "provider_affinity_header" }
)]
#[case::client_control(
    Scenario::RejectsClientField { request: r#"{"_litellm_control":{"stream_chunk_size":5}}"#, field: "_litellm_control" }
)]
#[case::client_credentials(
    Scenario::RejectsClientField { request: r#"{"_litellm_internal_model_credentials":{"aws_access_key_id":"client"}}"#, field: "_litellm_internal_model_credentials" }
)]
#[case::request_identity_reaches_affinity(Scenario::AcceptsRequestIdentity)]
#[case::request_metadata_cannot_replace_trusted_fields(
    Scenario::MetadataCannotReplaceTrustedFields
)]
#[case::explicit_credentials_keep_the_destination(Scenario::KeepsTrustedDestination)]
#[case::non_default_control_is_stored(Scenario::StoresNonDefaultControl)]
#[case::default_control_leaves_no_key(Scenario::DropsDefaultControl)]
#[case::trusted_credentials_are_carried(Scenario::CarriesTrustedCredentials)]
fn provider_controls_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::RejectsClientField { request, field } => {
            let error = serde_json::from_str::<RequestCall>(request)
                .expect_err("a request may not name a trusted field");
            assert!(error.to_string().contains(field));
        }
        Scenario::AcceptsRequestIdentity => {
            let request = serde_json::from_str::<RequestCall>(
                r#"{"session_id":"request-session","litellm_session_id":"request-litellm",
                   "metadata":{"session_id":"metadata-session"},"litellm_metadata":{"session_id":"litellm-metadata-session"}}"#,
            )
            .expect("the request identity fields decode");
            let params = call_params(&trusted_call(), &request);
            assert_eq!(
                params.affinity.session_id.as_deref(),
                Some("request-session")
            );
            assert_eq!(
                params.affinity.litellm_session_id.as_deref(),
                Some("request-litellm")
            );
            assert_eq!(
                params.affinity.provider_affinity_header.as_deref(),
                Some("x-session")
            );
            assert_eq!(
                params
                    .affinity
                    .metadata
                    .expect("metadata")
                    .session_id
                    .as_deref(),
                Some("metadata-session")
            );
            assert_eq!(
                params
                    .affinity
                    .litellm_metadata
                    .expect("litellm_metadata")
                    .session_id
                    .as_deref(),
                Some("litellm-metadata-session")
            );
        }
        Scenario::MetadataCannotReplaceTrustedFields => {
            let request = serde_json::from_str::<RequestCall>(
                r#"{"metadata":{"api_key":"client-key","api_base":"https://client.test","provider_affinity_header":"x-client"}}"#,
            )
            .expect("open metadata keys decode but stay metadata");
            let params = call_params(&trusted_call(), &request);
            assert_eq!(params.api_base.as_deref(), Some("https://configured.test"));
            assert_eq!(params.custom_llm_provider.as_deref(), Some("azure"));
            assert_eq!(
                params.api_key.as_ref().map(ExposeSecret::expose_secret),
                Some("server-key")
            );
            assert_eq!(
                params.affinity.provider_affinity_header.as_deref(),
                Some("x-session")
            );
        }
        Scenario::KeepsTrustedDestination => {
            let request =
                serde_json::from_str::<RequestCall>(r#"{"session_id":"request-session"}"#)
                    .expect("the request identity fields decode");
            let params = call_params(&trusted_call(), &request);
            assert_eq!(params.api_base.as_deref(), Some("https://configured.test"));
            assert_eq!(params.custom_llm_provider.as_deref(), Some("azure"));
            assert_eq!(params.model.as_deref(), Some("azure/gpt-4o"));
            assert_eq!(
                params.api_key.as_ref().map(ExposeSecret::expose_secret),
                Some("server-key")
            );
        }
        Scenario::StoresNonDefaultControl => {
            let params = call_params(&trusted_call(), &RequestCall::default());
            assert_eq!(
                params.control,
                Some(ControlOptions {
                    stream_chunk_size: Some(512)
                })
            );
        }
        Scenario::DropsDefaultControl => {
            let trusted = TrustedCall {
                model: None,
                api_key: None,
                api_base: None,
                custom_llm_provider: None,
                provider_affinity_header: None,
                control: ControlOptions::new(None).expect("a default control"),
                credentials: None,
            };
            let mut params = call_params(&trusted, &RequestCall::default());
            with_control_options(
                &mut params,
                ControlOptions {
                    stream_chunk_size: Some(7),
                },
            );
            assert_eq!(
                params.control,
                Some(ControlOptions {
                    stream_chunk_size: Some(7)
                })
            );
            with_control_options(&mut params, ControlOptions::default());
            assert_eq!(params.control, None);
        }
        Scenario::CarriesTrustedCredentials => {
            let mut values = BTreeMap::new();
            values.insert(
                "aws_secret_access_key".to_owned(),
                SecretString::from("credential-secret"),
            );
            let trusted = TrustedCall {
                model: None,
                api_key: Some(SecretString::from("server-key")),
                api_base: None,
                custom_llm_provider: None,
                provider_affinity_header: None,
                control: ControlOptions::default(),
                credentials: Some(TrustedCredentials { values }),
            };
            let params = call_params(&trusted, &RequestCall::default());
            let credentials = params
                .credentials
                .as_ref()
                .expect("credentials are carried");
            assert!(credentials.values.contains_key("aws_secret_access_key"));
            assert_eq!(
                TRUSTED_CREDENTIALS_KEY,
                "_litellm_internal_model_credentials"
            );
            let rendered = format!("{params:?}");
            assert!(!rendered.contains("credential-secret"));
            assert!(!rendered.contains("server-key"));
        }
    }
}

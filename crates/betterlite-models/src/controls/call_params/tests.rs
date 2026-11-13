#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};

use crate::controls::call_params::{
    DropParams, RequestCall, TRUSTED_CREDENTIALS_KEY, TrustedCall, TrustedCredentials, call_params,
    with_control_options,
};
use crate::controls::control_options::ControlOptions;
use crate::controls::optional_params::OptionalParams;

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
    DerivesSessionAndTraceIds {
        request: &'static str,
        session_id: &'static str,
        trace_id: &'static str,
    },
    ResolvesMetadata {
        request: &'static str,
        session_id: &'static str,
        trace_id: &'static str,
    },
    NormalizesDropParams {
        flag: Option<bool>,
        text: Option<&'static str>,
        expected: Option<bool>,
    },
    CarriesNoLog,
    CarriesTrustedOptionalKeys,
}

fn trusted_call() -> TrustedCall {
    let optional: OptionalParams =
        serde_json::from_str(r#"{"provider_affinity_header":"x-session"}"#)
            .expect("the trusted optional params decode");
    TrustedCall {
        model: Some("azure/gpt-4o".to_owned()),
        api_key: Some(SecretString::from("server-key")),
        api_base: Some("https://configured.test".to_owned()),
        custom_llm_provider: Some("azure".to_owned()),
        control: ControlOptions::new(Some(512)).expect("a valid control"),
        credentials: None,
        drop_params: None,
        no_log: None,
        optional,
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
#[case::client_no_log(
    Scenario::RejectsClientField { request: r#"{"no-log":true}"#, field: "no-log" }
)]
#[case::client_drop_params(
    Scenario::RejectsClientField { request: r#"{"drop_params":true}"#, field: "drop_params" }
)]
#[case::request_identity_reaches_affinity(Scenario::AcceptsRequestIdentity)]
#[case::request_metadata_cannot_replace_trusted_fields(
    Scenario::MetadataCannotReplaceTrustedFields
)]
#[case::explicit_credentials_keep_the_destination(Scenario::KeepsTrustedDestination)]
#[case::non_default_control_is_stored(Scenario::StoresNonDefaultControl)]
#[case::default_control_leaves_no_key(Scenario::DropsDefaultControl)]
#[case::trusted_credentials_are_carried(Scenario::CarriesTrustedCredentials)]
#[case::session_id_comes_from_metadata(
    Scenario::DerivesSessionAndTraceIds {
        request: r#"{"metadata":{"session_id":"meta-session"}}"#,
        session_id: "meta-session",
        trace_id: "meta-session",
    }
)]
#[case::trace_id_comes_from_metadata(
    Scenario::DerivesSessionAndTraceIds {
        request: r#"{"metadata":{"trace_id":"meta-trace"}}"#,
        session_id: "meta-trace",
        trace_id: "meta-trace",
    }
)]
#[case::session_and_trace_ids_keep_their_own_keys(
    Scenario::DerivesSessionAndTraceIds {
        request: r#"{"metadata":{"session_id":"meta-session","trace_id":"meta-trace"}}"#,
        session_id: "meta-session",
        trace_id: "meta-trace",
    }
)]
#[case::explicit_ids_win_over_metadata(
    Scenario::DerivesSessionAndTraceIds {
        request: r#"{"litellm_session_id":"explicit-session","litellm_trace_id":"explicit-trace",
                   "metadata":{"session_id":"meta-session","trace_id":"meta-trace"}}"#,
        session_id: "explicit-session",
        trace_id: "explicit-trace",
    }
)]
#[case::absent_metadata_falls_back_to_litellm_metadata(
    Scenario::ResolvesMetadata {
        request: r#"{"litellm_metadata":{"session_id":"fallback"}}"#,
        session_id: "fallback",
        trace_id: "fallback",
    }
)]
#[case::empty_metadata_falls_back_to_litellm_metadata(
    Scenario::ResolvesMetadata {
        request: r#"{"metadata":{},"litellm_metadata":{"session_id":"fallback"}}"#,
        session_id: "fallback",
        trace_id: "fallback",
    }
)]
#[case::non_empty_metadata_wins_over_litellm_metadata(
    Scenario::ResolvesMetadata {
        request: r#"{"metadata":{"session_id":"meta-session"},"litellm_metadata":{"session_id":"fallback"}}"#,
        session_id: "meta-session",
        trace_id: "meta-session",
    }
)]
#[case::drop_flag_true(Scenario::NormalizesDropParams { flag: Some(true), text: None, expected: Some(true) })]
#[case::drop_flag_false(Scenario::NormalizesDropParams { flag: Some(false), text: None, expected: Some(false) })]
#[case::drop_text_yes(Scenario::NormalizesDropParams { flag: None, text: Some(" YES "), expected: Some(true) })]
#[case::drop_text_off(Scenario::NormalizesDropParams { flag: None, text: Some("Off"), expected: Some(false) })]
#[case::drop_text_unreadable(Scenario::NormalizesDropParams { flag: None, text: Some("maybe"), expected: None })]
#[case::drop_absent(Scenario::NormalizesDropParams { flag: None, text: None, expected: None })]
#[case::no_log_is_carried(Scenario::CarriesNoLog)]
#[case::trusted_optional_keys_are_carried(Scenario::CarriesTrustedOptionalKeys)]
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
                control: ControlOptions::new(None).expect("a default control"),
                credentials: None,
                drop_params: None,
                no_log: None,
                optional: OptionalParams::default(),
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
                control: ControlOptions::default(),
                credentials: Some(TrustedCredentials { values }),
                drop_params: None,
                no_log: None,
                optional: OptionalParams::default(),
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
        Scenario::DerivesSessionAndTraceIds {
            request,
            session_id,
            trace_id,
        } => {
            let decoded = serde_json::from_str::<RequestCall>(request)
                .expect("the request identity fields decode");
            let params = call_params(&trusted_call(), &decoded);
            assert_eq!(params.litellm_session_id.as_deref(), Some(session_id));
            assert_eq!(params.litellm_trace_id.as_deref(), Some(trace_id));
        }
        Scenario::ResolvesMetadata {
            request,
            session_id,
            trace_id,
        } => {
            let decoded = serde_json::from_str::<RequestCall>(request)
                .expect("the request identity fields decode");
            let params = call_params(&trusted_call(), &decoded);
            assert_eq!(params.litellm_session_id.as_deref(), Some(session_id));
            assert_eq!(params.litellm_trace_id.as_deref(), Some(trace_id));
            assert_eq!(
                params
                    .metadata
                    .expect("resolved metadata")
                    .session_id
                    .as_deref(),
                Some(session_id)
            );
        }
        Scenario::NormalizesDropParams {
            flag,
            text,
            expected,
        } => {
            let mut trusted = trusted_call();
            trusted.drop_params = match (flag, text) {
                (Some(set_flag), None) => Some(DropParams::Flag(set_flag)),
                (None, Some(phrase)) => Some(DropParams::Text(phrase.to_owned())),
                _ => None,
            };
            let params = call_params(&trusted, &RequestCall::default());
            assert_eq!(params.drop_params, expected);
        }
        Scenario::CarriesNoLog => {
            let mut trusted = trusted_call();
            trusted.no_log = Some(true);
            let params = call_params(&trusted, &RequestCall::default());
            assert_eq!(params.no_log, Some(true));
        }
        Scenario::CarriesTrustedOptionalKeys => {
            let mut trusted = trusted_call();
            trusted.optional.timeout = Some(30.0_f64);
            trusted.optional.rpm = Some(600);
            trusted.optional.aws_access_key_id = Some(SecretString::from("aws-key"));
            trusted.optional.pricing.values.input.input_token = Some(Decimal::new(5, 6));
            let params = call_params(&trusted, &RequestCall::default());
            assert!(params.optional.timeout.is_some());
            assert_eq!(params.optional.rpm, Some(600));
            assert_eq!(
                params
                    .optional
                    .aws_access_key_id
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                Some("aws-key")
            );
            assert_eq!(
                params.optional.pricing.values.input.input_token,
                Some(Decimal::new(5, 6))
            );
            assert_eq!(params.optional.tpm, None);
            assert_eq!(
                params.affinity.provider_affinity_header.as_deref(),
                Some("x-session")
            );
        }
    }
}

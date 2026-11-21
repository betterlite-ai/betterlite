#![cfg(test)]
#![expect(
    clippy::too_many_lines,
    reason = "each named behavior test states one LiteLLM transcription scenario end to end"
)]
mod extra;

use super::call::{SuppliedClientKind, TranscriptionCall};
use super::dispatch::{TranscriptionAnswer, TranscriptionDispatch, TranscriptionProviderReply};
use super::error::{AsyncTranscriptionError, TranscriptionDispatchError, TranscriptionError};
use super::service::Transcriptions;
use super::target::{InjectedTranscription, TranscriptionInput, TranscriptionTarget};
use betterlite_config::sections::transcription_call::Settings;
use betterlite_context::media_config_selection::audio_transcription::AudioTranscriptionConfigTag;
use betterlite_context::non_default_params::Params;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::call_helpers::headers::ProviderHeaderProjection;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::audio_file_input::decoder::{
    AudioDecodeLimits, AudioDecoder, NativeAudioDecoder,
};
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, TupleContent,
};
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;
use uuid::Uuid;
const INIT: &str = r#"{"model":"whisper-1","call_type":"transcription","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[test]
fn transcription_openai_form() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        harness.dispatch.reply(response(
            r#"{"text":"Hello","words":[{"word":"Hello","start":0,"end":1}]}"#,
        ));
        let mut request = request_for("openai/whisper-1");
        request.language = Some(String::from("en"));
        request.prompt = Some(String::from("hint"));
        request.response_format = Some(String::from("verbose_json"));
        request.timestamp_granularities = Some(vec![String::from("word"), String::from("segment")]);
        request.temperature = Some(0);
        let audio = wav(1);
        let answer = harness
            .service()
            .transcription(
                &request,
                AudioFile::Tuple(AudioTuple {
                    filename: "a.wav",
                    content: Some(TupleContent::Bytes(&audio)),
                }),
            )
            .await
            .expect("the transcription succeeds");
        let recorded = harness.dispatch.targets();
        assert_eq!(recorded.len(), 1);
        let target = &recorded[0];
        assert_eq!(target.kind, TargetKind::OpenAi);
        assert_eq!(target.model, "whisper-1");
        assert_eq!(target.input, "(a.wav, bytes)");
        assert_eq!(target.filename, "a.wav");
        assert_eq!(target.file, audio);
        assert_eq!(
            target.api_base.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(target.api_key, None);
        assert_eq!(target.family, Some(TranscriptionFamily::Whisper));
        assert_eq!(target.organization, None);
        assert!(!target.atranscription);
        assert_eq!(target.timeout, Duration::from_secs(600));
        assert_eq!(target.max_retries, 2);
        assert!(!target.shared_session);
        assert_eq!(
            target.optional_params,
            params(
                r#"{"language":"en","prompt":"hint","response_format":"verbose_json",
                    "temperature":0,"timestamp_granularities":["word","segment"]}"#
            )
        );
        let TranscriptionAnswer::Reply(reply) = answer else {
            panic!("the handler answered a typed response");
        };
        assert_eq!(reply.response.text.as_deref(), Some("Hello"));
        assert!(reply.response.extra.get("words").is_some());
        assert!(reply.response.extra.get("duration").is_none());
        assert_eq!(hidden_duration(&reply.response), Some(1.0));
        // text and srt replies pass through, and no JSON response format is forced on the provider
        harness.dispatch.reply(response(
            r#"{"text":"1\n00:00:00,000 --> 00:00:01,000\nHello"}"#,
        ));
        let mut srt = request_for("openai/whisper-1");
        srt.response_format = Some(String::from("srt"));
        let srt_answer = harness
            .service()
            .transcription(&srt, AudioFile::Bytes(&audio))
            .await
            .expect("the srt transcription succeeds");
        let srt_targets = harness.dispatch.targets();
        assert_eq!(srt_targets.len(), 2);
        assert_eq!(
            srt_targets[1].optional_params,
            params(r#"{"response_format":"srt"}"#)
        );
        assert_eq!(srt_targets[1].input, "audio.wav");
        let TranscriptionAnswer::Reply(srt_reply) = srt_answer else {
            panic!("the srt handler answered a typed response");
        };
        assert_eq!(
            srt_reply.response.text.as_deref(),
            Some("1\n00:00:00,000 --> 00:00:01,000\nHello")
        );
    });
}

#[test]
fn transcription_openai_retry_defaults() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        let audio = wav(1);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut zero = request_for("openai/whisper-1");
        zero.max_retries = Some(0);
        harness
            .service()
            .transcription(&zero, AudioFile::Bytes(&audio))
            .await
            .expect("the transcription succeeds");
        assert_eq!(harness.dispatch.targets()[0].max_retries, 0);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut global = request_for("openai/whisper-1");
        global.params = params(r#"{"num_retries":9}"#);
        harness
            .service()
            .transcription(&global, AudioFile::Bytes(&audio))
            .await
            .expect("the transcription succeeds");
        assert_eq!(harness.dispatch.targets()[1].max_retries, 2);
    });
}

#[test]
fn transcription_credential_precedence() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        harness.settings.api_base = Some(String::from("https://global.test/v1"));
        harness.settings.api_key = Some(secret("global-key"));
        harness.settings.openai_key = Some(secret("openai-key"));
        harness.settings.openai_api_key = Some(secret("env-key"));
        harness.settings.organization = Some(String::from("global-org"));
        harness.settings.openai_organization = Some(String::from("env-org"));
        let audio = wav(1);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut openai = request_for("openai/whisper-1");
        openai.api_base = Some(String::from("https://call.test/v1"));
        openai.api_key = Some(secret("call-key"));
        harness
            .service()
            .transcription(&openai, AudioFile::Bytes(&audio))
            .await
            .expect("the transcription succeeds");
        let first = harness.dispatch.last();
        assert_eq!(first.api_base.as_deref(), Some("https://call.test/v1"));
        assert_eq!(first.api_key.as_deref(), Some("call-key"));
        assert_eq!(first.organization.as_deref(), Some("global-org"));
        // a dynamic empty key replaces the call key, and the branch chain then skips it
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut empty = request_for("openai/whisper-1");
        empty.api_key = Some(secret(""));
        harness
            .service()
            .transcription(&empty, AudioFile::Bytes(&audio))
            .await
            .expect("the transcription succeeds");
        let second = harness.dispatch.last();
        assert_eq!(second.api_key.as_deref(), Some("global-key"));
        assert_eq!(second.api_base.as_deref(), Some("https://global.test/v1"));
        // base_url stands in for a missing api_base
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut from_base_url = request_for("openai/whisper-1");
        from_base_url.base_url = Some(String::from("https://base-url.test/v1"));
        harness
            .service()
            .transcription(&from_base_url, AudioFile::Bytes(&audio))
            .await
            .expect("the transcription succeeds");
        assert_eq!(
            harness.dispatch.last().api_base.as_deref(),
            Some("https://base-url.test/v1")
        );
        // the environment-key fallbacks come after the globals, and OPENAI_ORGANIZATION after litellm.organization
        harness.settings.api_key = None;
        harness.settings.openai_key = None;
        harness.settings.organization = Some(String::new());
        harness.settings.api_base = None;
        harness.settings.openai_base_url = Some(String::from("https://env-base.test/v1"));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let plain = request_for("openai/whisper-1");
        harness
            .service()
            .transcription(&plain, AudioFile::Bytes(&audio))
            .await
            .expect("the transcription succeeds");
        let fourth = harness.dispatch.last();
        assert_eq!(fourth.api_key.as_deref(), Some("env-key"));
        assert_eq!(fourth.api_base.as_deref(), Some("https://env-base.test/v1"));
        assert_eq!(fourth.organization.as_deref(), Some("env-org"));
        // azure: base, version, token and key each take their own call/global/env order
        let mut azure = Harness::new();
        azure.settings.api_base = Some(String::from("https://litellm-base.test"));
        azure.settings.api_version = Some(String::from("2023-09-01"));
        azure.settings.api_key = Some(secret("litellm-key"));
        azure.settings.azure_key = Some(secret("azure-key"));
        azure.settings.azure_api_key = Some(secret("env-azure-key"));
        azure.settings.azure_openai_api_key = Some(secret("env-azure-openai-key"));
        azure.settings.azure_ad_token = Some(secret("env-token"));
        azure.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut azure_call = request_for("azure/whisper");
        azure_call.api_version = Some(String::from("2024-06-01"));
        azure_call.azure_ad_token = Some(secret("call-token"));
        azure_call.extra_headers = Some(header_map(&[("x-caller", "yes")]));
        azure_call.params = params(r#"{"azure_ad_token":"call-token"}"#);
        azure
            .service()
            .transcription(&azure_call, AudioFile::Bytes(&audio))
            .await
            .expect("the azure transcription succeeds");
        let azure_target = azure.dispatch.last();
        assert_eq!(azure_target.kind, TargetKind::Azure);
        assert_eq!(
            azure_target.api_base.as_deref(),
            Some("https://litellm-base.test")
        );
        assert_eq!(azure_target.api_version.as_deref(), Some("2024-06-01"));
        assert_eq!(azure_target.api_key.as_deref(), Some("litellm-key"));
        assert_eq!(azure_target.azure_ad_token.as_deref(), Some("call-token"));
        assert_eq!(
            azure_target.optional_params.get("extra_headers"),
            Some(&value(r#"{"x-caller":"yes"}"#))
        );
        assert_eq!(
            azure_target.litellm_params.get("azure_ad_token"),
            Some(&value(r#""call-token""#))
        );
        // an empty call key falls through, and AZURE_OPENAI_API_KEY alone never fills this branch's key
        let mut env_only = Harness::new();
        env_only.settings.azure_openai_api_key = Some(secret("env-azure-openai-key"));
        env_only.settings.azure_ad_token = Some(secret("env-token"));
        env_only.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut empty_key = request_for("azure/whisper");
        empty_key.api_key = Some(secret(""));
        env_only
            .service()
            .transcription(&empty_key, AudioFile::Bytes(&audio))
            .await
            .expect("the azure transcription succeeds");
        let env_target = env_only.dispatch.last();
        assert_eq!(env_target.api_key, None);
        assert_eq!(
            env_target.optional_params.get("extra_headers"),
            Some(&JsonValue::Null)
        );
        assert_eq!(env_target.azure_ad_token.as_deref(), Some("env-token"));
    });
}

#[test]
fn transcription_native_selection() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        let audio = wav(1);
        // xai's config opts into its own native endpoint, so the caller's headers and client are dropped
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut xai = request_for("xai/whisper");
        xai.extra_headers = Some(header_map(&[("x-caller", "yes")]));
        xai.client = Some(SuppliedClientKind::OpenAi);
        harness
            .service()
            .transcription(&xai, AudioFile::Bytes(&audio))
            .await
            .expect("the native transcription succeeds");
        let native = harness.dispatch.last();
        assert_eq!(native.kind, TargetKind::Native);
        assert_eq!(native.config, Some(AudioTranscriptionConfigTag::XAI));
        assert_eq!(native.headers, Some(Headers::new()));
        assert_eq!(native.client, None);
        // azure with an azure_speech config is native too
        harness.catalog = catalog(
            r#"{"azure/azure-speech":{"litellm_provider":"azure","mode":"audio_transcription",
                "audio_transcription_config":"azure_speech"}}"#,
        );
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let speech = request_for("azure/azure-speech");
        harness
            .service()
            .transcription(&speech, AudioFile::Bytes(&audio))
            .await
            .expect("the azure speech transcription succeeds");
        let speech_target = harness.dispatch.last();
        assert_eq!(speech_target.kind, TargetKind::Native);
        assert_eq!(
            speech_target.config,
            Some(AudioTranscriptionConfigTag::AzureSpeech)
        );
        // soniox keeps the caller headers and only the HTTP handler clients
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut soniox = request_for("soniox/stt");
        soniox.extra_headers = Some(header_map(&[("x-caller", "yes")]));
        soniox.client = Some(SuppliedClientKind::HttpHandler);
        harness
            .service()
            .transcription(&soniox, AudioFile::Bytes(&audio))
            .await
            .expect("the soniox transcription succeeds");
        let soniox_target = harness.dispatch.last();
        assert_eq!(soniox_target.kind, TargetKind::Soniox);
        assert_eq!(
            soniox_target.headers,
            Some(header_map(&[("x-caller", "yes")]))
        );
        assert_eq!(soniox_target.client, Some(SuppliedClientKind::HttpHandler));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut soniox_sdk = request_for("soniox/stt");
        soniox_sdk.client = Some(SuppliedClientKind::OpenAi);
        harness
            .service()
            .transcription(&soniox_sdk, AudioFile::Bytes(&audio))
            .await
            .expect("the soniox transcription succeeds");
        assert_eq!(harness.dispatch.last().client, None);
        // riva reaches its own operation, and bedrock's atranscription flag travels with its headers
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let riva = request_for("nvidia_riva/stt");
        harness
            .service()
            .transcription(&riva, AudioFile::Bytes(&audio))
            .await
            .expect("the riva transcription succeeds");
        assert_eq!(harness.dispatch.last().kind, TargetKind::NvidiaRiva);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut bedrock = request_for("bedrock/stt");
        bedrock.atranscription = Some(Json::Bool(true));
        bedrock.extra_headers = Some(header_map(&[("x-caller", "yes")]));
        harness
            .service()
            .transcription(&bedrock, AudioFile::Bytes(&audio))
            .await
            .expect("the bedrock transcription succeeds");
        let bedrock_target = harness.dispatch.last();
        assert_eq!(bedrock_target.kind, TargetKind::Bedrock);
        assert!(bedrock_target.atranscription);
        assert_eq!(
            bedrock_target.headers,
            Some(header_map(&[("x-caller", "yes")]))
        );
        // a provider with no branch raises before any fabricated successful result
        let count = harness.dispatch.targets().len();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let unmapped = request_for("cohere/command");
        let Err(unmapped_error) = harness
            .service()
            .transcription(&unmapped, AudioFile::Bytes(&audio))
            .await
        else {
            panic!("the provider is unmapped");
        };
        assert_eq!(
            unmapped_error.to_string(),
            "Unmapped provider passed in. Unable to get the response."
        );
        assert_eq!(harness.dispatch.targets().len(), count);
        // watsonx answers the unbuilt entry instead of falling through to another arm
        let watsonx = request_for("watsonx/ibm-granite");
        let Err(watsonx_error) = harness
            .service()
            .transcription(&watsonx, AudioFile::Bytes(&audio))
            .await
        else {
            panic!("watsonx has no injected operation");
        };
        assert_eq!(
            watsonx_error.to_string(),
            "betterlite does not dispatch the watsonx_transcription call yet"
        );
        assert_eq!(harness.dispatch.targets().len(), count);
        // a provider 429 keeps B011's rate-limit class and status through the async wrapper
        harness.dispatch.failure();
        let limited = request_for("openai/whisper-1");
        let Err(limited_error) = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &limited,
                AudioFile::Bytes(&audio),
                true,
            )
            .await
        else {
            panic!("the provider rate limited the call");
        };
        let AsyncTranscriptionError::Exception(exception) = limited_error else {
            panic!("a provider failure maps to an exception");
        };
        assert!(matches!(exception.class, ErrorClass::RateLimit(_)));
        assert_eq!(exception.class.metadata().status_code, 429_u16);
        assert_eq!(
            exception.class.metadata().provider.as_deref(),
            Some("openai")
        );
    });
}

#[test]
fn transcription_async_results() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        let audio = wav(1);
        let request = request_for("openai/whisper-1");
        // a missing model is a KeyError raised before the try
        let Err(missing_error) = harness
            .service()
            .atranscription(None, &request, AudioFile::Bytes(&audio), true)
            .await
        else {
            panic!("the model is missing");
        };
        assert!(matches!(
            missing_error,
            AsyncTranscriptionError::MissingModel
        ));
        assert!(harness.dispatch.targets().is_empty());
        // a cached dict is built by the typed constructor
        harness.dispatch.mapping(json(r#"{"text":"Hello"}"#));
        let converted = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&audio),
                true,
            )
            .await
            .expect("the cached dict converts");
        assert_eq!(converted.text.as_deref(), Some("Hello"));
        assert_eq!(harness.dispatch.targets().len(), 1);
        assert!(harness.dispatch.targets()[0].atranscription);
        assert_eq!(hidden_duration(&converted), Some(1.0));
        assert!(converted.extra.get("duration").is_none());
        // the outer duration step needs the file in kwargs, which a cached dict does not get from the sync stage
        harness.dispatch.mapping(json(r#"{"text":"Hello"}"#));
        let positional_mapping = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&audio),
                false,
            )
            .await
            .expect("the cached dict converts");
        assert_eq!(hidden_duration(&positional_mapping), None);
        // a typed response passes straight through, and the sync stage measures it even without kwargs
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let typed = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&audio),
                false,
            )
            .await
            .expect("the typed response passes through");
        assert_eq!(typed.text.as_deref(), Some("Hello"));
        assert_eq!(hidden_duration(&typed), Some(1.0));
        harness.dispatch.unexpected(json("1"));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let second = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&audio),
                true,
            )
            .await
            .expect("the second call answers a typed response");
        assert_eq!(second.text.as_deref(), Some("Hello"));
        // a final integer raises the wrapper's ValueError text
        harness.dispatch.unexpected(json("1"));
        harness.dispatch.unexpected(json("1"));
        let Err(invalid_error) = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&audio),
                true,
            )
            .await
        else {
            panic!("an integer is not a transcription response");
        };
        let AsyncTranscriptionError::Exception(exception) = invalid_error else {
            panic!("the final check maps to an exception");
        };
        assert_eq!(
            exception.message,
            "Invalid response from transcription provider, expected TranscriptionResponse, but got <class 'int'>"
        );
        // a null public duration is absent, so the fallback stores the hidden value
        harness
            .dispatch
            .reply(response(r#"{"text":"Hello","duration":null}"#));
        let null_duration = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&audio),
                true,
            )
            .await
            .expect("the transcription succeeds");
        assert_eq!(hidden_duration(&null_duration), Some(1.0));
        // a zero-duration valid file stores zero, and invalid audio stores nothing
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let silent = wav(0);
        let zero = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&silent),
                true,
            )
            .await
            .expect("a zero-duration file is valid");
        assert_eq!(hidden_duration(&zero), Some(0.0));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let invalid = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(b"not a wav"),
                true,
            )
            .await
            .expect("invalid audio still transcribes");
        assert_eq!(hidden_duration(&invalid), None);
        // an unavailable decoder stores nothing either
        harness.decoder = None;
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let undecoded = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&audio),
                true,
            )
            .await
            .expect("the transcription succeeds without a decoder");
        assert_eq!(hidden_duration(&undecoded), None);
    });
}

#[rstest]
#[case("0")]
#[case("2")]
fn public_duration_is_preserved(#[case] existing: &str) {
    runtime().block_on(async {
        let mut harness = Harness::new();
        let audio = wav(1);
        let request = request_for("openai/whisper-1");
        harness.dispatch.reply(response(&format!(
            r#"{{"text":"Hello","duration":{existing}}}"#
        )));
        let preserved = harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&audio),
                true,
            )
            .await
            .expect("the public duration is preserved");
        assert_eq!(hidden_duration(&preserved), None);
        assert!(preserved.extra.get("duration").is_some());
    });
}

#[test]
fn cached_dict_conversion() {
    let converted = super::cached_audio_response(json(
        r#"{"text":"Hello","duration":2,"_hidden_params":{"audio_transcription_duration":2.0}}"#,
    ))
    .expect("the cached dict converts");
    assert_eq!(converted.text.as_deref(), Some("Hello"));
    assert!(converted.extra.get("duration").is_some());
    assert_eq!(hidden_duration(&converted), Some(2.0));
    let null_text =
        super::cached_audio_response(json(r#"{"text":null}"#)).expect("a null text converts");
    assert_eq!(null_text.text, None);
    assert!(matches!(
        super::cached_audio_response(json(r#"{"text":5}"#)),
        Err(TranscriptionError::CachedField { .. })
    ));
    assert!(matches!(
        super::cached_audio_response(json("[1]")),
        Err(TranscriptionError::CachedResponseNotObject)
    ));
}
#[rstest]
#[case("null", "<class 'NoneType'>")]
#[case("true", "<class 'bool'>")]
#[case("1", "<class 'int'>")]
#[case("1.5", "<class 'float'>")]
#[case("\"x\"", "<class 'str'>")]
#[case("[]", "<class 'list'>")]
#[case("{}", "<class 'dict'>")]
fn error_type_names(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(super::error_type_name(&json(source)), expected);
}
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    kind: TargetKind,
    model: String,
    input: String,
    filename: String,
    file: Vec<u8>,
    api_base: Option<String>,
    api_key: Option<String>,
    client: Option<SuppliedClientKind>,
    optional_params: Params,
    litellm_params: Params,
    atranscription: bool,
    timeout: Duration,
    max_retries: u32,
    shared_session: bool,
    boundary: Uuid,
    organization: Option<String>,
    family: Option<TranscriptionFamily>,
    api_version: Option<String>,
    azure_ad_token: Option<String>,
    headers: Option<Headers>,
    config: Option<AudioTranscriptionConfigTag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    OpenAi,
    Azure,
    NvidiaRiva,
    Soniox,
    Bedrock,
    Native,
}

impl Snapshot {
    fn of(kind: TargetKind, input: TranscriptionInput<'_>) -> Self {
        Self {
            kind,
            model: input.model.to_owned(),
            input: input.input.to_owned(),
            filename: input.file.filename.clone(),
            file: input.file.file_content.clone(),
            api_base: input.api_base.map(str::to_owned),
            api_key: input.api_key.map(|key| key.expose_secret().to_owned()),
            client: input.client,
            optional_params: input.optional_params.clone(),
            litellm_params: input.litellm_params.clone(),
            atranscription: input.atranscription,
            timeout: input.timeout,
            max_retries: input.max_retries,
            shared_session: input.shared_session.is_some(),
            boundary: input.boundary,
            organization: None,
            family: None,
            api_version: None,
            azure_ad_token: None,
            headers: None,
            config: None,
        }
    }
}

fn snapshot(target: TranscriptionTarget<'_>) -> Snapshot {
    match target {
        TranscriptionTarget::OpenAi(call) => {
            let mut recorded = Snapshot::of(TargetKind::OpenAi, call.input);
            recorded.organization = call.organization.map(str::to_owned);
            recorded.family = call.family;
            recorded
        }
        TranscriptionTarget::Azure(call) => {
            let mut recorded = Snapshot::of(TargetKind::Azure, call.input);
            recorded.api_version = call.api_version.map(str::to_owned);
            recorded.azure_ad_token = call
                .azure_ad_token
                .map(|token| token.expose_secret().to_owned());
            recorded
        }
        TranscriptionTarget::NvidiaRiva(call) => injected(TargetKind::NvidiaRiva, call),
        TranscriptionTarget::Soniox(call) => injected(TargetKind::Soniox, call),
        TranscriptionTarget::Bedrock(call) => injected(TargetKind::Bedrock, call),
        TranscriptionTarget::Native(call) => injected(TargetKind::Native, call),
        TranscriptionTarget::Watsonx(call) => injected(TargetKind::Native, call),
    }
}

fn injected(kind: TargetKind, call: InjectedTranscription<'_>) -> Snapshot {
    let mut recorded = Snapshot::of(kind, call.input);
    recorded.headers = call.headers.cloned();
    recorded.config = call.config;
    recorded
}

#[derive(Debug, thiserror::Error)]
#[error("rate limited by the provider")]
struct FakeError;

impl TranscriptionDispatchError for FakeError {
    fn status_code(&self) -> Option<u16> {
        Some(429)
    }
    fn error_str(&self) -> String {
        String::from("rate limited by the provider")
    }
}

#[derive(Default)]
struct Recorder {
    targets: Mutex<Vec<Snapshot>>,
    answers: Mutex<VecDeque<Result<TranscriptionAnswer, FakeError>>>,
}

impl Recorder {
    fn push(&self, answer: TranscriptionAnswer) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Ok(answer));
    }
    fn reply(&self, body: TranscriptionResponse) {
        self.push(TranscriptionAnswer::Reply(Box::new(
            TranscriptionProviderReply {
                response: body,
                provider_headers: ProviderHeaderProjection::default(),
            },
        )));
    }
    fn mapping(&self, body: Json) {
        self.push(TranscriptionAnswer::Mapping(body));
    }
    fn unexpected(&self, body: Json) {
        self.push(TranscriptionAnswer::Unexpected(body));
    }
    fn failure(&self) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Err(FakeError));
    }
    fn targets(&self) -> Vec<Snapshot> {
        self.targets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn last(&self) -> Snapshot {
        self.targets()
            .pop()
            .expect("the dispatch recorded a target")
    }
}

impl TranscriptionDispatch for Recorder {
    type Error = FakeError;
    fn transcribe(
        &self,
        target: TranscriptionTarget<'_>,
    ) -> impl Future<Output = Result<TranscriptionAnswer, FakeError>> + Send {
        self.targets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(snapshot(target));
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(FakeError));
        ready(answer)
    }
}

struct Files;

impl AuthorizedAudioFiles for Files {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }
    fn open(&self, path: &Path) -> io::Result<Box<dyn io::Read + Send>> {
        Err(io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }
}

struct Harness {
    settings: Settings,
    environment: BTreeMap<String, String>,
    catalog: ModelCatalog,
    bundled: ModelCatalog,
    names: BTreeSet<String>,
    files: Files,
    decoder: Option<NativeAudioDecoder>,
    redactor: Redactor,
    logging: Mutex<LoggingState>,
    dispatch: Recorder,
}

impl Harness {
    fn new() -> Self {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor");
        let logging = LoggingState::new(
            serde_json::from_str::<LoggingInit>(INIT).expect("the logging init"),
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("trace"),
            Duration::default,
        )
        .expect("the logging state");
        Self {
            settings: Settings::default(),
            environment: BTreeMap::new(),
            catalog: ModelCatalog::default(),
            bundled: ModelCatalog::default(),
            names: BTreeSet::new(),
            files: Files,
            decoder: Some(NativeAudioDecoder {
                limits: AudioDecodeLimits::default(),
            }),
            redactor,
            logging: Mutex::new(logging),
            dispatch: Recorder::default(),
        }
    }
    fn service(&self) -> Transcriptions<'_, Recorder> {
        Transcriptions {
            dispatch: &self.dispatch,
            settings: &self.settings,
            resolution: ResolutionContext {
                catalog: &self.catalog,
                known_model_names: &self.names,
                custom_providers: &self.names,
                environment: &self.environment,
                registry: JSONProviderRegistry::default(),
                policies: ProviderPolicies::default(),
                globals: GlobalCredentials::default(),
            },
            bundled: &self.bundled,
            files: &self.files,
            decoder: match &self.decoder {
                Some(decoder) => AudioDecoder::Available(decoder),
                None => AudioDecoder::Unavailable,
            },
            redactor: &self.redactor,
            logging: Some(&self.logging),
            identify: fixed_boundary,
        }
    }
}

fn fixed_boundary() -> Uuid {
    Uuid::from_u128(7)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime")
}

fn request_for(model: &str) -> TranscriptionCall {
    TranscriptionCall {
        model: model.to_owned(),
        ..TranscriptionCall::default()
    }
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn header_map(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("a params map")
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a JSON value")
}

fn value(source: &str) -> JsonValue {
    serde_json::from_str(source).expect("a JSON value")
}

fn catalog(source: &str) -> ModelCatalog {
    ModelCatalog::parse(source.as_bytes(), UnknownFields::Warn).expect("the catalog")
}

fn response(source: &str) -> TranscriptionResponse {
    let Json::Object(fields) = json(source) else {
        panic!("a transcription response is an object");
    };
    let mut extra = Object::new();
    let mut text = None;
    for (name, field) in fields {
        if name == "text" {
            if let Json::Text(value) = field {
                text = Some(value);
            }
            continue;
        }
        extra.insert(
            name,
            serde_json::to_value(field).expect("the extra field is JSON"),
        );
    }
    TranscriptionResponse { text, extra }
}

fn hidden_duration(body: &TranscriptionResponse) -> Option<f64> {
    let hidden = body.extra.get("_hidden_params")?.as_object()?;
    hidden.get("audio_transcription_duration")?.as_f64()
}
// a canonical 16-bit mono PCM WAV, so the duration decoder measures exactly the requested seconds
fn wav(seconds: u32) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 8000;
    let frames = SAMPLE_RATE * seconds;
    let data_len = frames * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.resize(
        bytes.len() + usize::try_from(data_len).expect("the data length"),
        0,
    );
    bytes
}

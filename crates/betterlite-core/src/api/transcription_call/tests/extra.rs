use std::sync::PoisonError;
use std::time::Duration;

use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::mapper::DropPolicy;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{AudioFile, ProcessedAudioFile};
use betterlite_openai::transcription::config::TranscriptionFamily;
use indexmap::IndexMap;

use super::support::request_for;
use super::{Harness, TargetKind, fixed_boundary, json, params, response, runtime, value, wav};
use crate::api::transcription_call::call::SuppliedClientKind;
use crate::api::transcription_call::error::{
    AsyncTranscriptionError, TranscriptionDispatchError, TranscriptionError,
};
use crate::api::transcription_call::plan::{bool_value, drop_policy};
use crate::api::transcription_call::target::TranscriptionInput;

#[derive(Debug, thiserror::Error)]
#[error("bare dispatch failure")]
struct BareError;

impl TranscriptionDispatchError for BareError {}

// the trait's own defaults, which every operation error that overrides nothing reports through
#[test]
fn dispatch_error_defaults() {
    assert_eq!(BareError.status_code(), None);
    assert_eq!(BareError.error_str(), "bare dispatch failure");
    assert_eq!(BareError.message(), "bare dispatch failure");
}

// kwargs.get("drop_params") counts only a literal bool, as the typed kwargs contract says
#[test]
fn drop_params_reads_a_literal_bool() {
    assert_eq!(bool_value(&value("true")), Some(true));
    assert_eq!(bool_value(&value("false")), Some(false));
    assert_eq!(bool_value(&value("null")), None);
    assert_eq!(bool_value(&value("1")), None);
    assert_eq!(bool_value(&value("\"true\"")), None);
}

// the kwargs bool and litellm_settings both feed the drop policy, and only a literal bool counts
#[test]
fn drop_policy_reads_settings_and_kwargs() {
    let harness = Harness::new();
    let mut call = request_for("xai/whisper");
    assert_eq!(
        drop_policy(&harness.settings, &call),
        DropPolicy {
            drop_params: None,
            litellm_drop_params: false,
        }
    );
    call.params = params(r#"{"drop_params":true}"#);
    assert_eq!(
        drop_policy(&harness.settings, &call),
        DropPolicy {
            drop_params: Some(true),
            litellm_drop_params: false,
        }
    );
    call.params = params(r#"{"drop_params":false}"#);
    assert_eq!(
        drop_policy(&harness.settings, &call),
        DropPolicy {
            drop_params: Some(false),
            litellm_drop_params: false,
        }
    );
    call.params = params(r#"{"drop_params":"true"}"#);
    assert_eq!(
        drop_policy(&harness.settings, &call),
        DropPolicy {
            drop_params: None,
            litellm_drop_params: false,
        }
    );
    let mut configured = Harness::new();
    configured.settings.drop_params = Some(true);
    call.params = Params::new();
    assert_eq!(
        drop_policy(&configured.settings, &call),
        DropPolicy {
            drop_params: None,
            litellm_drop_params: true,
        }
    );
}

#[test]
fn optional_params_map_keeps_the_decisions_shape() {
    let processed = ProcessedAudioFile {
        file_content: Vec::new(),
        filename: String::from("a.wav"),
        content_type: String::from("audio/wav"),
    };
    let input = TranscriptionInput {
        model: "whisper-1",
        input: "a.wav",
        file: &processed,
        optional_params: params(r#"{"language":"en","temperature":0}"#),
        litellm_params: Params::new(),
        atranscription: false,
        timeout: Duration::from_secs(600),
        max_retries: 2,
        api_base: None,
        api_key: None,
        client: None,
        shared_session: None,
        boundary: fixed_boundary(),
    };
    let mapped = input.optional_params_map();
    assert_eq!(
        mapped.get("language"),
        Some(&Json::Text(String::from("en")))
    );
    assert_eq!(mapped.get("temperature"), Some(&json("0")));
}

// model_call_details["client"] = str(client), with "None" when the caller passed no client
#[test]
fn logging_records_the_client_string() {
    runtime().block_on(async {
        let harness = Harness::new();
        let audio = wav(1);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut request = request_for("openai/whisper-1");
        request.params = params(r#"{"litellm_call_id":"caller-call"}"#);
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&audio))
            .await
            .expect("the transcription succeeds");
        let state = harness
            .logging
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            state.model_call_details.get("client"),
            Some(&Json::Text(String::from("None")))
        );
        assert!(state.optional_params.is_empty());
        // the caller's kwargs come after the six defaults, so its value replaces the default
        assert_eq!(
            state.litellm_params.get("litellm_call_id"),
            Some(&Json::Text(String::from("caller-call")))
        );
        assert_eq!(
            state.litellm_params.get("proxy_server_request"),
            Some(&Json::Null)
        );
        assert_eq!(
            state.litellm_params.get("stream_response"),
            Some(&Json::Object(IndexMap::new()))
        );
    });
}

// the generic native handler keeps only the two HTTPHandler client classes
#[test]
fn native_http_keeps_the_http_handler_client() {
    runtime().block_on(async {
        let harness = Harness::new();
        let audio = wav(1);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut request = request_for("xai/whisper");
        request.client = Some(SuppliedClientKind::HttpHandler);
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&audio))
            .await
            .expect("the native transcription succeeds");
        assert_eq!(
            harness.dispatch.last().client,
            Some(SuppliedClientKind::HttpHandler)
        );
    });
}

// a timeout Duration cannot hold is refused before any provider operation runs
#[test]
fn invalid_timeout_is_refused_before_dispatch() {
    runtime().block_on(async {
        let harness = Harness::new();
        let mut request = request_for("openai/whisper-1");
        request.timeout = Some(-1.0);
        let refused = harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await;
        let Err(error) = refused else {
            panic!("a negative timeout is refused");
        };
        assert!(matches!(error, TranscriptionError::InvalidTimeout { .. }));
        assert!(harness.dispatch.targets().is_empty());
    });
}

// tags, atranscription and client are popped before the branch sees the remaining kwargs
#[test]
fn popped_kwargs_never_reach_the_provider() {
    runtime().block_on(async {
        let harness = Harness::new();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut request = request_for("openai/whisper-1");
        request.params =
            params(r#"{"tags":["a"],"atranscription":true,"client":"x","azure_ad_token":"t"}"#);
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the transcription succeeds");
        let recorded = harness.dispatch.last();
        assert!(!recorded.litellm_params.contains_key("tags"));
        assert!(!recorded.litellm_params.contains_key("atranscription"));
        assert!(!recorded.litellm_params.contains_key("client"));
        assert_eq!(
            recorded.litellm_params.get("azure_ad_token"),
            Some(&value(r#""t""#))
        );
    });
}

// args[0] names the model ahead of the call's own field, and a missing one is the wrapper's KeyError
#[test]
fn atranscription_uses_the_first_argument_as_the_model() {
    runtime().block_on(async {
        let harness = Harness::new();
        let request = request_for("openai/gpt-4o-transcribe");
        let refusal = harness
            .service()
            .atranscription(None, &request, AudioFile::Bytes(&wav(1)), true)
            .await;
        let Err(absent_model) = refusal else {
            panic!("the model is missing");
        };
        assert!(matches!(
            absent_model,
            AsyncTranscriptionError::MissingModel
        ));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&wav(1)),
                true,
            )
            .await
            .expect("the positional model wins");
        assert_eq!(harness.dispatch.last().model, "whisper-1");
    });
}

// an unresolvable model is mapped with "openai" as the provider fallback the wrapper uses
#[test]
fn atranscription_falls_back_to_openai_for_an_unresolvable_model() {
    runtime().block_on(async {
        let harness = Harness::new();
        let request = request_for("no-such-provider/model-x");
        let answer = harness
            .service()
            .atranscription(
                Some("no-such-provider/model-x"),
                &request,
                AudioFile::Bytes(&wav(1)),
                true,
            )
            .await;
        let Err(refusal) = answer else {
            panic!("the provider is unknown");
        };
        let AsyncTranscriptionError::Exception(exception) = refusal else {
            panic!("the resolution failure maps to an exception");
        };
        assert_eq!(
            exception.class.metadata().provider.as_deref(),
            Some("openai")
        );
        assert!(harness.dispatch.targets().is_empty());
    });
}

// every openai-compatible provider joins "openai" in the transport set
#[test]
fn compatible_providers_use_the_openai_transport() {
    runtime().block_on(async {
        let harness = Harness::new();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let request = request_for("groq/whisper-large-v3");
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the compatible provider uses the openai transport");
        assert_eq!(harness.dispatch.last().kind, TargetKind::OpenAi);
    });
}

// an openai gpt-4o model selects the gpt family, where whisper's models select their own
#[test]
fn gpt_transcription_selects_the_gpt_family() {
    runtime().block_on(async {
        let harness = Harness::new();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let request = request_for("openai/gpt-4o-transcribe");
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the gpt transcription succeeds");
        assert_eq!(
            harness.dispatch.last().family,
            Some(TranscriptionFamily::Gpt)
        );
    });
}

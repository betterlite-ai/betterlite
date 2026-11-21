use std::sync::PoisonError;
use std::time::Duration;

use betterlite_context::non_default_params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{AudioFile, ProcessedAudioFile};

use super::{Harness, fixed_boundary, json, params, request_for, response, runtime, value, wav};
use crate::api::transcription_call::call::SuppliedClientKind;
use crate::api::transcription_call::error::{TranscriptionDispatchError, TranscriptionError};
use crate::api::transcription_call::plan::bool_value;
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
        let request = request_for("openai/whisper-1");
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
        let error = harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await;
        let Err(error) = error else {
            panic!("a negative timeout is refused");
        };
        assert!(matches!(error, TranscriptionError::InvalidTimeout { .. }));
        assert!(harness.dispatch.targets().is_empty());
    });
}

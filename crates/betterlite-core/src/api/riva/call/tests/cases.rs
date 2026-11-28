use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_pyhost::host::PythonHost;
use pyo3::types::PyAnyMethods as _;
use rstest::rstest;

use super::{
    CLIENT_BROKEN, CLIENT_WITHOUT_CONFIGS, PYTHON, RecordingLogger, Setup, audio_host, call,
    configure, decimal, install, install_fakes, json, modules, probe, reset, service, stage, value,
};
use crate::api::riva::call::error::{INSTALL_HINT, MISSING_ENDPOINT, RivaCallError};
use crate::api::riva::call::transcribe_attached;
use crate::api::transcription_call::error::TranscriptionDispatchError as _;

const FINAL_SPEC: &str = r#"[{"results": [
    {"is_final": true, "alternatives": [{"transcript": "hello "}, {"transcript": "ignored"}]},
    {"is_final": true, "alternatives": [{"transcript": "world"}]}
]}]"#;

const RECOGNITION_DEFAULT: &str = r#"{"audio_channel_count":1,"enable_automatic_punctuation":true,"enable_word_time_offsets":false,"encoding":1,"language_code":"en-US","max_alternatives":1,"model":"","profanity_filter":false,"sample_rate_hertz":16000,"verbatim_transcripts":false}"#;

const RECOGNITION_WITH_FILTERS: &str = r#"{"audio_channel_count":1,"enable_automatic_punctuation":false,"enable_word_time_offsets":true,"encoding":1,"language_code":"de-DE","max_alternatives":3,"model":"parakeet","profanity_filter":true,"sample_rate_hertz":16000,"verbatim_transcripts":true}"#;

const RECOGNITION_LOGGED: &str = r#"{"audio_channel_count":1,"enable_automatic_punctuation":true,"enable_word_time_offsets":false,"encoding":"LINEAR_PCM","language_code":"en-US","max_alternatives":1,"model":"","profanity_filter":false,"sample_rate_hertz":16000,"verbatim_transcripts":false}"#;

const AUTH_DEFAULT: &str = r#"{"metadata_args":[["authorization","Bearer riva-key"]],"uri":"grpc.example:443","use_ssl":false}"#;

const AUTH_NVCF: &str = r#"{"metadata_args":[["function-id","fn-123"],["authorization","Bearer riva-key"]],"uri":"grpc.nvcf.nvidia.com:443","use_ssl":true}"#;

const AUTH_NVCF_NO_KEY: &str = r#"{"metadata_args":[["function-id","fn-123"]],"uri":"grpc.nvcf.nvidia.com:443","use_ssl":true}"#;

const AUTH_ARGS: &str = r#"[null,true,"grpc.nvcf.nvidia.com:443",[["function-id","fn-123"],["authorization","Bearer riva-key"]]]"#;

const AUTH_TLS_ON: &str = r#"{"metadata_args":[["authorization","Bearer riva-key"]],"uri":"localhost:50051","use_ssl":true}"#;

const AUTH_TLS_OFF: &str = r#"{"metadata_args":[["authorization","Bearer riva-key"]],"uri":"localhost:50051","use_ssl":false}"#;

const AUTH_TLS_NVCF_OFF: &str = r#"{"metadata_args":[["function-id","fn-123"],["authorization","Bearer riva-key"]],"uri":"grpc.nvcf.nvidia.com:443","use_ssl":false}"#;

fn run_sync(
    host: &PythonHost,
    setup: &Setup,
    api_base: Option<&str>,
    atranscription: bool,
) -> (Result<TranscriptionReply, RivaCallError>, RecordingLogger) {
    let mut logger = RecordingLogger::default();
    let pending = call(setup, api_base, atranscription);
    let outcome = host.attach(|py| transcribe_attached(py, &pending, &mut logger));
    (outcome, logger)
}

fn complete(
    host: &PythonHost,
    optional: &str,
    api_base: &str,
) -> (TranscriptionReply, RecordingLogger) {
    reset(host, 1600);
    stage(host, FINAL_SPEC);
    let setup = Setup::new(optional);
    let (outcome, logger) = run_sync(host, &setup, Some(api_base), false);
    (outcome.unwrap(), logger)
}

fn failure(host: &PythonHost, flags: &[&str], kind: Option<&str>, specs: &str) -> RivaCallError {
    reset(host, 1600);
    stage(host, specs);
    configure(host, flags);
    if let Some(selected) = kind {
        service(host, selected);
    }
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(host, &setup, Some("grpc.example:443"), false);
    outcome.unwrap_err()
}

fn auth_logger(host: &PythonHost, optional: &str, api_base: &str) -> (String, RecordingLogger) {
    let (_, logger) = complete(host, optional, api_base);
    (value(&probe(host), "auth.kwargs"), logger)
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn missing_endpoint_is_rejected_before_any_sdk_work(#[case] endpoint: Option<&str>) {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    let setup = Setup::new("{}");
    let mut logger = RecordingLogger::default();
    let pending = call(&setup, endpoint, false);
    let error = host
        .attach(|py| transcribe_attached(py, &pending, &mut logger))
        .unwrap_err();
    assert_eq!(error.to_string(), MISSING_ENDPOINT);
    assert_eq!(error.status_code(), Some(400));
    let recorded = probe(&host);
    assert!(recorded.events.is_empty());
}

#[test]
fn sdk_call_matches_the_source_protocol() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 3250);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    outcome.unwrap();
    let recorded = probe(&host);
    assert_eq!(
        recorded.events,
        vec!["auth", "recognition", "streaming", "asr", "stream.call"]
    );
    assert_eq!(value(&recorded, "auth.kwargs"), AUTH_DEFAULT);
    assert_eq!(value(&recorded, "auth.args"), "[]");
    assert_eq!(value(&recorded, "recognition.kwargs"), RECOGNITION_DEFAULT);
    assert_eq!(value(&recorded, "streaming.interim_results"), "false");
    assert_eq!(
        value(&recorded, "streaming.keys"),
        r#"["config","interim_results"]"#
    );
    assert_eq!(value(&recorded, "stream.timeout"), "30.0");
    assert_eq!(recorded.chunk_sizes, vec![vec![3200_usize, 3200, 100]]);
}

#[test]
fn sdk_call_builds_the_source_reply() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 3250);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let reply = outcome.unwrap();
    assert_eq!(reply.response.text.as_deref(), Some("hello world"));
    assert_eq!(
        reply.response.extra.get("task"),
        Some(&serde_json::json!("transcribe"))
    );
    assert_eq!(reply.model.as_deref(), Some("nvidia-riva/parakeet"));
    assert_eq!(reply.custom_llm_provider, "nvidia_riva");
    assert_eq!(
        reply.duration,
        StoredDuration::Known(Some(decimal("0.203125")))
    );
    assert_eq!(
        reply.original_response,
        json(r#"{"text":"hello world","task":"transcribe"}"#)
    );
}

#[test]
fn sdk_call_logs_match_the_source() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 3250);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new("{}");
    let (outcome, logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let reply = outcome.unwrap();
    assert_eq!(logger.pre.len(), 1);
    assert_eq!(logger.pre[0].api_base, "grpc.example:443");
    assert!(!logger.pre[0].atranscription);
    assert!(!logger.pre[0].nvcf_function_id_set);
    assert_eq!(logger.pre[0].use_ssl, Json::Null);
    assert_eq!(
        Json::Object(logger.pre[0].recognition_config.clone()),
        json(RECOGNITION_LOGGED)
    );
    assert_eq!(logger.post.len(), 1);
    assert_eq!(logger.post[0].input, json(r#""clip.wav""#));
    assert_eq!(logger.post[0].original_response, reply.original_response);
    assert_eq!(
        Json::Object(logger.post[0].recognition_config.clone()),
        json(RECOGNITION_LOGGED)
    );
}

#[test]
fn nvcf_function_id_defaults_to_tls_and_adds_metadata() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new(r#"{"nvcf_function_id":"fn-123"}"#);
    let (outcome, logger) = run_sync(&host, &setup, Some("grpc.nvcf.nvidia.com:443"), false);
    outcome.unwrap();
    let recorded = probe(&host);
    assert_eq!(value(&recorded, "auth.kwargs"), AUTH_NVCF);
    assert!(logger.pre[0].nvcf_function_id_set);
    assert_eq!(logger.pre[0].use_ssl, Json::Null);

    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    let keyless = Setup::new(r#"{"nvcf_function_id":"fn-123"}"#).without_key();
    let (keyless_outcome, _keyless_logger) =
        run_sync(&host, &keyless, Some("grpc.nvcf.nvidia.com:443"), false);
    keyless_outcome.unwrap();
    assert_eq!(value(&probe(&host), "auth.kwargs"), AUTH_NVCF_NO_KEY);
}

#[test]
fn self_hosted_tls_follows_the_override() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    let (flag_true, flag_true_logger) =
        auth_logger(&host, r#"{"use_ssl":true}"#, "localhost:50051");
    assert_eq!(flag_true, AUTH_TLS_ON);
    assert_eq!(flag_true_logger.pre[0].use_ssl, json("true"));
    let (quoted_yes, quoted_yes_logger) =
        auth_logger(&host, r#"{"use_ssl":"yes"}"#, "localhost:50051");
    assert_eq!(quoted_yes, AUTH_TLS_ON);
    assert_eq!(quoted_yes_logger.pre[0].use_ssl, json(r#""yes""#));
    let (empty_list, empty_list_logger) =
        auth_logger(&host, r#"{"use_ssl":[]}"#, "localhost:50051");
    assert_eq!(empty_list, AUTH_TLS_OFF);
    assert_eq!(empty_list_logger.pre[0].use_ssl, json("[]"));
    let (empty_object, empty_object_logger) =
        auth_logger(&host, r#"{"use_ssl":{}}"#, "localhost:50051");
    assert_eq!(empty_object, AUTH_TLS_OFF);
    assert_eq!(empty_object_logger.pre[0].use_ssl, json("{}"));
    let (zero_number, zero_number_logger) = auth_logger(
        &host,
        r#"{"use_ssl":0,"nvcf_function_id":"fn-123"}"#,
        "grpc.nvcf.nvidia.com:443",
    );
    assert_eq!(zero_number, AUTH_TLS_NVCF_OFF);
    assert_eq!(zero_number_logger.pre[0].use_ssl, json("0"));
    assert!(zero_number_logger.pre[0].nvcf_function_id_set);
}

#[test]
fn legacy_auth_signature_is_retried_positionally() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    configure(&host, &["auth_legacy"]);
    let setup = Setup::new(r#"{"nvcf_function_id":"fn-123"}"#);
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.nvcf.nvidia.com:443"), false);
    outcome.unwrap();
    let recorded = probe(&host);
    assert_eq!(
        recorded.events,
        vec![
            "auth",
            "auth",
            "recognition",
            "streaming",
            "asr",
            "stream.call"
        ]
    );
    assert_eq!(value(&recorded, "auth.kwargs"), AUTH_NVCF);
    assert_eq!(value(&recorded, "auth.args"), AUTH_ARGS);
}

#[test]
fn recognition_options_convert_integers_and_bools() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    let (reply, logger) = complete(
        &host,
        r#"{"language_code":"de-DE","riva_model_name":"parakeet","max_alternatives":3,"enable_automatic_punctuation":false,"enable_word_time_offsets":true,"verbatim_transcripts":true,"profanity_filter":true}"#,
        "grpc.example:443",
    );
    assert_eq!(reply.response.text.as_deref(), Some("hello world"));
    let recorded = probe(&host);
    assert_eq!(
        value(&recorded, "recognition.kwargs"),
        RECOGNITION_WITH_FILTERS
    );
    assert_eq!(
        Json::Object(logger.pre[0].recognition_config.clone()),
        json(
            r#"{"audio_channel_count":1,"enable_automatic_punctuation":false,"enable_word_time_offsets":true,"encoding":"LINEAR_PCM","language_code":"de-DE","max_alternatives":3,"model":"parakeet","profanity_filter":true,"sample_rate_hertz":16000,"verbatim_transcripts":true}"#
        )
    );
}

#[test]
fn wire_format_overrides_the_caller_settings() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    let (_, logger) = complete(
        &host,
        r#"{"sample_rate_hertz":8000,"audio_channel_count":2,"encoding":"flac"}"#,
        "grpc.example:443",
    );
    let recorded = probe(&host);
    assert_eq!(value(&recorded, "recognition.kwargs"), RECOGNITION_DEFAULT);
    assert_eq!(
        Json::Object(logger.pre[0].recognition_config.clone()),
        json(RECOGNITION_LOGGED)
    );
}

#[test]
fn the_named_encoding_constant_is_read_from_the_sdk() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    host.attach(|py| {
        let sdk = py.import("riva_call_sdk").unwrap();
        sdk.getattr("AudioEncoding")
            .unwrap()
            .setattr("LINEAR_PCM", 7_i32)
            .unwrap();
    });
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    outcome.unwrap();
    let recorded = probe(&host);
    assert_eq!(
        value(&recorded, "recognition.kwargs"),
        RECOGNITION_DEFAULT.replace(r#""encoding":1"#, r#""encoding":7"#)
    );
}

#[test]
fn a_missing_linear_pcm_constant_fails_the_config() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    host.attach(|py| {
        let sdk = py.import("riva_call_sdk").unwrap();
        sdk.getattr("AudioEncoding")
            .unwrap()
            .delattr("LINEAR_PCM")
            .unwrap();
    });
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        RivaCallError::Config {
            operation: "riva.client.RecognitionConfig",
            ..
        }
    ));
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn auth_and_recognition_failures_keep_the_python_cause() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    let auth_error = failure(&host, &["auth_fails"], None, FINAL_SPEC);
    assert!(matches!(
        auth_error,
        RivaCallError::Config {
            operation: "riva.client.Auth",
            ..
        }
    ));
    assert!(
        auth_error
            .to_string()
            .starts_with("riva.client.Auth failed")
    );
    assert_eq!(auth_error.status_code(), Some(500));
    let auth_only = probe(&host);
    assert_eq!(auth_only.events, vec!["auth"]);

    let type_error = failure(&host, &["auth_type_error"], None, FINAL_SPEC);
    assert!(matches!(type_error, RivaCallError::Config { .. }));
    let retried = probe(&host);
    assert_eq!(retried.events, vec!["auth", "auth"]);

    let recognition_error = failure(&host, &["recognition_fails"], None, FINAL_SPEC);
    assert!(matches!(
        recognition_error,
        RivaCallError::Config {
            operation: "riva.client.RecognitionConfig",
            ..
        }
    ));
    assert!(
        recognition_error
            .to_string()
            .starts_with("riva.client.RecognitionConfig failed")
    );
}

#[test]
fn endpointing_is_copied_into_the_config() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    let (_, logger) = complete(
        &host,
        r#"{"endpointing_config":{"start_history":10,"stop_history":500}}"#,
        "grpc.example:443",
    );
    let recorded = probe(&host);
    assert_eq!(
        recorded.events,
        vec![
            "auth",
            "recognition",
            "endpointing",
            "endpointing.copy",
            "streaming",
            "asr",
            "stream.call"
        ]
    );
    assert_eq!(
        value(&recorded, "endpointing.kwargs"),
        r#"{"start_history":10,"stop_history":500}"#
    );
    assert_eq!(
        value(&recorded, "endpointing.copied"),
        r#"{"start_history":10,"stop_history":500}"#
    );
    assert_eq!(
        Json::Object(logger.pre[0].recognition_config.clone()),
        json(
            r#"{"audio_channel_count":1,"enable_automatic_punctuation":true,"enable_word_time_offsets":false,"encoding":"LINEAR_PCM","endpointing_config":{"start_history":10,"stop_history":500},"language_code":"en-US","max_alternatives":1,"model":"","profanity_filter":false,"sample_rate_hertz":16000,"verbatim_transcripts":false}"#
        )
    );
}

#[rstest]
#[case::init_fails(&["endpointing_init_fails"][..])]
#[case::copy_fails(&["endpointing_copy_fails"][..])]
#[case::field_missing(&["endpointing_field_missing"][..])]
fn endpointing_failures_leave_the_riva_defaults(#[case] flags: &[&str]) {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    let endpointing = r#"{"endpointing_config":{"start_history":10}}"#;
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    configure(&host, flags);
    let setup = Setup::new(endpointing);
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    outcome.unwrap();

    let (_, _empty_logger) = complete(&host, r#"{"endpointing_config":{}}"#, "grpc.example:443");
    let empty_probe = probe(&host);
    assert!(
        !empty_probe
            .events
            .iter()
            .any(|event| event == "endpointing")
    );

    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    host.attach(|py| {
        py.import("riva.client")
            .unwrap()
            .delattr("EndpointingConfig")
            .unwrap();
    });
    let missing_setup = Setup::new(endpointing);
    let (missing_outcome, _missing_logger) =
        run_sync(&host, &missing_setup, Some("grpc.example:443"), false);
    missing_outcome.unwrap();
    let missing_probe = probe(&host);
    assert!(
        !missing_probe
            .events
            .iter()
            .any(|event| event == "endpointing")
    );
}

#[rstest]
#[case::no_samples(0, Vec::new())]
#[case::one_sample(1, vec![2])]
#[case::one_chunk(1600, vec![3200])]
#[case::chunk_plus_one(1601, vec![3200, 2])]
#[case::two_chunks(3200, vec![3200, 3200])]
#[case::three_chunks(4800, vec![3200, 3200, 3200])]
fn audio_is_streamed_in_3200_byte_slices(#[case] samples: usize, #[case] expected: Vec<usize>) {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, samples);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    outcome.unwrap();
    let recorded = probe(&host);
    assert_eq!(recorded.chunk_sizes, vec![expected], "{samples} samples");
}

#[rstest]
#[case::timeout_kwarg("timeout", "30.0", None)]
#[case::variadic("variadic", "30.0", Some(r#"["timeout"]"#))]
#[case::plain("plain", "absent", None)]
#[case::broken("broken", "absent", None)]
fn timeout_is_sent_only_when_the_generator_takes_it(
    #[case] kind: &str,
    #[case] expected: &str,
    #[case] extra: Option<&str>,
) {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    service(&host, kind);
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    outcome.unwrap();
    let recorded = probe(&host);
    assert_eq!(value(&recorded, "stream.timeout"), expected, "{kind}");
    if let Some(sent) = extra {
        assert_eq!(value(&recorded, "stream.extra"), sent, "{kind}");
    }
}

#[test]
fn provider_failures_map_through_the_grpc_table() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    let asr_error = failure(&host, &["asr_fails"], None, FINAL_SPEC);
    assert!(matches!(asr_error, RivaCallError::Provider(_)));
    assert_eq!(asr_error.to_string(), "NVIDIA Riva error: asr refused");
    assert_eq!(asr_error.status_code(), Some(500));
    let grpc_error = failure(&host, &["grpc_failure"], None, FINAL_SPEC);
    assert_eq!(
        grpc_error.to_string(),
        "NVIDIA Riva gRPC error (UNAVAILABLE): backend down"
    );
    assert_eq!(grpc_error.status_code(), Some(503));
    let generator_error = failure(&host, &[], Some("raises"), FINAL_SPEC);
    assert_eq!(
        generator_error.to_string(),
        "NVIDIA Riva error: generator refused"
    );
    let scalar_error = failure(&host, &[], Some("scalar"), FINAL_SPEC);
    assert_eq!(scalar_error.status_code(), Some(500));
    let entry_error = failure(&host, &["entry_fails"], None, FINAL_SPEC);
    assert_eq!(entry_error.to_string(), "NVIDIA Riva error: stream refused");
    let iteration_error = failure(&host, &[], None, r#"[{"raise":"iteration refused"}]"#);
    assert_eq!(
        iteration_error.to_string(),
        "NVIDIA Riva error: iteration refused"
    );
    let attribute_error = failure(&host, &[], None, r#"[{"raises":["results"]}]"#);
    assert_eq!(
        attribute_error.to_string(),
        "NVIDIA Riva error: attribute results failed"
    );
}

#[test]
fn final_only_chunks_are_aggregated_like_the_source() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(
        &host,
        r#"[
            {"id": 5},
            {"results": []},
            {"results": [
                {"is_final": false, "alternatives": [{"transcript": "partial"}]},
                {"is_final": true, "alternatives": []},
                {"is_final": true},
                {"is_final": true, "alternatives": [{"transcript": ""}]},
                {"is_final": true, "alternatives": [{"transcript": " hello "}, {"transcript": "second"}]}
            ]}
        ]"#,
    );
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let reply = outcome.unwrap();
    assert_eq!(reply.response.text.as_deref(), Some("hello"));
    assert_eq!(
        reply.original_response,
        json(r#"{"text":"hello","task":"transcribe"}"#)
    );
}

#[test]
fn word_timings_convert_integers_and_falsey_values() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(
        &host,
        r#"[{"results": [{"is_final": true, "alternatives": [{"transcript": "hi", "words": [
            {"word": "a", "start_time": 1000, "end_time": 1500},
            {"word": "b", "start_time": 1500.9, "end_time": -1500.9},
            {"word": "c", "start_time": true, "end_time": "2000"},
            {"word": "d", "start_time": null, "end_time": false},
            {"word": "e"}
        ]}]}]}]"#,
    );
    let setup =
        Setup::new(r#"{"response_format":"verbose_json","timestamp_granularities":["word"]}"#);
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let reply = outcome.unwrap();
    assert_eq!(reply.response.text.as_deref(), Some("hi"));
    let expected: Object = match serde_json::from_str(
        r#"{"words":[{"word":"a","start":1.0,"end":1.5},{"word":"b","start":1.5,"end":-1.5},{"word":"c","start":0.001,"end":2.0},{"word":"d","start":0.0,"end":0.0},{"word":"e","start":0.0,"end":0.0}],"duration":0.1}"#,
    ) {
        Ok(parsed) => parsed,
        Err(error) => panic!("the expected word timings are invalid: {error}"),
    };
    assert_eq!(reply.response.extra.get("words"), expected.get("words"));
    assert_eq!(
        reply.response.extra.get("duration"),
        expected.get("duration")
    );
}

#[test]
fn malformed_stream_values_are_rejected() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    let transcript_error = failure(
        &host,
        &[],
        None,
        r#"[{"results": [{"is_final": true, "alternatives": [{"transcript": 5}]}]}]"#,
    );
    assert!(matches!(transcript_error, RivaCallError::Malformed { .. }));
    assert_eq!(transcript_error.status_code(), Some(500));
    let word_error = failure(
        &host,
        &[],
        None,
        r#"[{"results": [{"is_final": true, "alternatives": [{"transcript": "x", "words": [{"word": 5}]}]}]}]"#,
    );
    assert!(matches!(word_error, RivaCallError::Malformed { .. }));
    let timing_error = failure(
        &host,
        &[],
        None,
        r#"[{"results": [{"is_final": true, "alternatives": [{"transcript": "x", "words": [{"word": "w", "start_time": "x"}]}]}]}]"#,
    );
    assert!(matches!(timing_error, RivaCallError::Malformed { .. }));
}

#[test]
fn the_proto_module_supplies_missing_constructors() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    install_fakes(&host, 1600, CLIENT_WITHOUT_CONFIGS, true);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let reply = outcome.unwrap();
    assert_eq!(reply.response.text.as_deref(), Some("hello world"));
    let recorded = probe(&host);
    assert_eq!(value(&recorded, "recognition.kwargs"), RECOGNITION_DEFAULT);
}

#[test]
fn a_missing_client_returns_the_install_hint() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    install_fakes(&host, 1600, CLIENT_WITHOUT_CONFIGS, false);
    stage(&host, FINAL_SPEC);
    let no_configs = Setup::new("{}");
    let (configs_outcome, _configs_logger) =
        run_sync(&host, &no_configs, Some("grpc.example:443"), false);
    let missing_constructor = configs_outcome.unwrap_err();
    assert!(matches!(missing_constructor, RivaCallError::Install { .. }));
    assert_eq!(missing_constructor.to_string(), INSTALL_HINT);
    assert_eq!(missing_constructor.status_code(), Some(500));

    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    host.attach(|py| {
        let registered = modules(py);
        drop(registered.del_item("riva"));
        drop(registered.del_item("riva.client"));
        drop(registered.del_item("riva.client.proto"));
        drop(registered.del_item("riva.client.proto.riva_asr_pb2"));
    });
    let absent_setup = Setup::new("{}");
    let (absent_outcome, _absent_logger) =
        run_sync(&host, &absent_setup, Some("grpc.example:443"), false);
    let absent_client = absent_outcome.unwrap_err();
    assert!(matches!(absent_client, RivaCallError::Install { .. }));
    assert_eq!(absent_client.to_string(), INSTALL_HINT);
}

#[test]
fn a_broken_client_attribute_keeps_the_cause() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    install_fakes(&host, 1600, CLIENT_BROKEN, true);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let error = outcome.unwrap_err();
    assert!(matches!(error, RivaCallError::Import { .. }));
    assert!(error.to_string().contains("client attribute failed"));
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn audio_errors_keep_their_status() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 0);
    let source = "class _Info:\n    frames = 16777217\n    channels = 1\n    samplerate = 16000\ndef info(file):\n    return _Info()\ndef read(file, dtype='float32', always_2d=False):\n    raise RuntimeError('read must not run')\n";
    host.attach(|py| {
        let module = install(py, "soundfile", source);
        modules(py).set_item("soundfile", &module).unwrap();
    });
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let error = outcome.unwrap_err();
    assert!(matches!(error, RivaCallError::Audio(_)));
    assert_eq!(error.status_code(), Some(413));
    let recorded = probe(&host);
    assert!(recorded.events.is_empty());
}

#[test]
fn record_failures_name_the_option() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new(r#"{"max_alternatives":"two"}"#);
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let error = outcome.unwrap_err();
    assert!(matches!(error, RivaCallError::Records(_)));
    assert_eq!(
        error.to_string(),
        "the max_alternatives option must be a non-negative integer"
    );
    assert_eq!(error.status_code(), Some(400));
    let recorded = probe(&host);
    assert!(recorded.events.is_empty());
}

#[test]
fn a_non_string_function_id_names_the_field() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    let setup = Setup::new(r#"{"nvcf_function_id":5}"#);
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let error = outcome.unwrap_err();
    assert!(matches!(error, RivaCallError::Records(_)));
    assert_eq!(
        error.to_string(),
        "the nvcf_function_id option must be a string"
    );
    assert_eq!(error.status_code(), Some(400));
    let recorded = probe(&host);
    assert!(recorded.events.is_empty());
}

#[test]
fn a_named_audio_file_is_logged_unchanged() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    let mut setup = Setup::new("{}");
    setup.input = String::from("meeting-notes.wav");
    let (outcome, logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    outcome.unwrap();
    assert_eq!(logger.post[0].input, json(r#""meeting-notes.wav""#));
}

const CLIENT_IMPORT_SOURCE: &str = "def __getattr__(name):
    raise RuntimeError('riva import failed')
";

const PROTO_IMPORT_SOURCE: &str = "def __getattr__(name):
    raise RuntimeError('proto import failed')
";

const SIGNATURE_SOURCE: &str = "def signature(obj):
    raise RuntimeError('signature refused')
";

const ENCODING_SOURCE: &str = "class _Later:
    def __init__(self, error):
        self.calls = 0
        self.error = error

    def __getattr__(self, name):
        if name != 'LINEAR_PCM':
            raise AttributeError(name)
        self.calls += 1
        if self.calls > 1:
            raise self.error
        return 1


missing = _Later(AttributeError('LINEAR_PCM'))
failing = _Later(RuntimeError('encoding refused'))
";

fn replace_encoding(host: &PythonHost, which: &str) {
    host.attach(|py| {
        let module = install(py, "riva_call_encoding", ENCODING_SOURCE);
        let replacement = module.getattr(which).unwrap();
        py.import("riva.client")
            .unwrap()
            .setattr("AudioEncoding", replacement)
            .unwrap();
    });
}

#[test]
fn a_client_import_failure_that_is_not_an_import_error_keeps_its_cause() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    host.attach(|py| {
        let raising = install(py, "riva", CLIENT_IMPORT_SOURCE);
        let registered = modules(py);
        registered.set_item("riva", &raising).unwrap();
        drop(registered.del_item("riva.client"));
    });
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let error = outcome.unwrap_err();
    assert!(matches!(error, RivaCallError::Import { .. }));
    assert!(error.to_string().contains("riva import failed"));
}

#[test]
fn a_proto_import_failure_that_is_not_an_import_error_keeps_its_cause() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    install_fakes(&host, 1600, CLIENT_WITHOUT_CONFIGS, true);
    stage(&host, FINAL_SPEC);
    host.attach(|py| {
        let raising = install(py, "riva.client.proto", PROTO_IMPORT_SOURCE);
        let registered = modules(py);
        registered.set_item("riva.client.proto", &raising).unwrap();
        drop(registered.del_item("riva.client.proto.riva_asr_pb2"));
    });
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let error = outcome.unwrap_err();
    assert!(matches!(error, RivaCallError::Import { .. }));
    assert!(error.to_string().contains("proto import failed"));
}

#[test]
fn a_signature_failure_that_is_not_a_type_error_maps_through_the_grpc_table() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    let original = host.attach(|py| {
        let inspect = py.import("inspect").unwrap();
        let previous = inspect.getattr("signature").unwrap().unbind();
        let patched = install(py, "riva_call_inspect", SIGNATURE_SOURCE)
            .getattr("signature")
            .unwrap();
        inspect.setattr("signature", patched).unwrap();
        previous
    });
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    host.attach(|py| {
        py.import("inspect")
            .unwrap()
            .setattr("signature", original.bind(py))
            .unwrap();
    });
    let error = outcome.unwrap_err();
    assert!(matches!(error, RivaCallError::Provider(_)));
    assert_eq!(error.to_string(), "NVIDIA Riva error: signature refused");
}

#[test]
fn a_named_encoding_missing_on_a_later_read_falls_back_to_linear_pcm() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    replace_encoding(&host, "missing");
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    outcome.unwrap();
    let recorded = probe(&host);
    assert_eq!(value(&recorded, "recognition.kwargs"), RECOGNITION_DEFAULT);
}

#[test]
fn a_named_encoding_failure_that_is_not_missing_names_the_config() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = audio_host() else {
        return;
    };
    reset(&host, 1600);
    stage(&host, FINAL_SPEC);
    replace_encoding(&host, "failing");
    let setup = Setup::new("{}");
    let (outcome, _logger) = run_sync(&host, &setup, Some("grpc.example:443"), false);
    let error = outcome.unwrap_err();
    assert!(matches!(
        error,
        RivaCallError::Config {
            operation: "riva.client.RecognitionConfig",
            ..
        }
    ));
}

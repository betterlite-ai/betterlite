#![cfg(test)]

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;
use http::HeaderMap;

use super::{AudioFile, AudioTranscriptionConfig, TranscriptionContractError};

struct Base;

impl AudioTranscriptionConfig for Base {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }
}

struct Capabilities {
    native: bool,
    subtitles: bool,
}

impl AudioTranscriptionConfig for Capabilities {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn has_native_endpoint(&self) -> bool {
        self.native
    }

    fn supports_subtitle_synthesis(&self) -> bool {
        self.subtitles
    }
}

struct Supported {
    names: Vec<String>,
}

impl AudioTranscriptionConfig for Supported {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.names.clone()
    }
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("object")
}

fn supported(names: &[&str]) -> Supported {
    Supported {
        names: names.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn record(data: AudioTranscriptionData) -> AudioTranscriptionRequestData {
    AudioTranscriptionRequestData {
        data,
        files: None,
        content_type: None,
    }
}

fn audio() -> AudioFile<'static> {
    AudioFile {
        filename: "a.wav",
        content: b"abc",
        content_type: "audio/wav",
    }
}

fn given_options() -> Object {
    object(
        r#"{"model":"m","OPENAI_TRANSCRIPTION_PARAMS":"internal","language":"en","response_format":"json","diarize":false,"count":0,"label":"","unset":null,"extra":"yes"}"#,
    )
}

#[test]
fn transcription_contract_defaults() {
    assert_request_defaults();
    assert_explicit_request_fields();
    assert_capability_defaults(&Base);
    assert_url_resolution(&Base);
    assert_transform_diagnostics(&Base);
}

#[test]
fn transcription_provider_parameters() {
    let config = supported(&["language", "response_format"]);
    assert_filters_provider_parameters(&config);
    assert_supported_names_remove_parameters();
    assert_openai_params_are_not_consulted(&config);
    assert_reserved_names_are_case_sensitive(&config);
    assert_options_are_not_mutated(&config);
}

fn assert_request_defaults() {
    let object_record = record(AudioTranscriptionData::Object(Object::new()));
    assert_eq!(
        object_record.data,
        AudioTranscriptionData::Object(Object::new())
    );
    assert!(object_record.files.is_none());
    assert!(object_record.content_type.is_none());
    assert_eq!(
        serde_json::to_string(&object_record).expect("encode"),
        r#"{"data":{}}"#
    );
    let bytes_record = record(AudioTranscriptionData::Bytes(b"abc".to_vec()));
    assert_eq!(
        bytes_record.data,
        AudioTranscriptionData::Bytes(b"abc".to_vec())
    );
    assert_eq!(bytes_record.files, None);
    assert_eq!(bytes_record.content_type, None);
    assert_eq!(
        serde_json::to_string(&bytes_record).expect("encode"),
        r#"{"data":[97,98,99]}"#
    );
}

fn assert_explicit_request_fields() {
    let explicit = AudioTranscriptionRequestData {
        data: AudioTranscriptionData::Object(Object::new()),
        files: Some(AudioTranscriptionFiles::new()),
        content_type: Some(String::from("audio/wav")),
    };
    assert_eq!(explicit.files, Some(AudioTranscriptionFiles::new()));
    assert_eq!(explicit.content_type.as_deref(), Some("audio/wav"));
    let named = AudioTranscriptionRequestData {
        data: AudioTranscriptionData::Object(Object::new()),
        files: Some(AudioTranscriptionFiles::from([(
            String::from("a.wav"),
            AudioTranscriptionFileValue::Text(String::from("abc")),
        )])),
        content_type: None,
    };
    let found = named
        .files
        .as_ref()
        .and_then(|files| files.get("a.wav"))
        .cloned();
    assert_eq!(
        found,
        Some(AudioTranscriptionFileValue::Text(String::from("abc")))
    );
}

fn assert_capability_defaults<C>(config: &C)
where
    C: AudioTranscriptionConfig,
{
    assert!(!config.has_native_endpoint());
    assert!(!config.supports_subtitle_synthesis());
    let native_only = Capabilities {
        native: true,
        subtitles: false,
    };
    assert!(native_only.has_native_endpoint());
    assert!(!native_only.supports_subtitle_synthesis());
    let subtitles_only = Capabilities {
        native: false,
        subtitles: true,
    };
    assert!(!subtitles_only.has_native_endpoint());
    assert!(subtitles_only.supports_subtitle_synthesis());
}

fn assert_url_resolution<C>(config: &C)
where
    C: AudioTranscriptionConfig,
{
    let empty = Object::new();
    assert_eq!(config.complete_url(None, "m", &empty, &empty), "");
    assert_eq!(config.complete_url(Some(""), "m", &empty, &empty), "");
    assert_eq!(
        config.complete_url(Some("https://mock/transcribe"), "m", &empty, &empty),
        "https://mock/transcribe"
    );
}

fn assert_transform_diagnostics<C>(config: &C)
where
    C: AudioTranscriptionConfig,
{
    let empty = Object::new();
    let missing = config
        .transform_audio_request("m", &audio(), &empty, &empty)
        .unwrap_err();
    assert_eq!(missing, TranscriptionContractError::MissingRequestTransform);
    assert_eq!(
        missing.to_string(),
        "AudioTranscriptionConfig needs a request transformation for audio transcription models"
    );
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: Vec::new(),
    };
    let unexpected = config.transform_audio_response(&response).unwrap_err();
    assert_eq!(
        unexpected,
        TranscriptionContractError::UnexpectedResponseTransform
    );
    assert_eq!(
        unexpected.to_string(),
        "AudioTranscriptionConfig does not need a response transformation for audio transcription models"
    );
}

fn assert_filters_provider_parameters(config: &Supported) {
    let kept = config.provider_specific_params("m", &given_options(), &[String::from("extra")]);
    assert_eq!(
        kept,
        object(r#"{"diarize":false,"count":0,"label":"","extra":"yes"}"#)
    );
}

fn assert_supported_names_remove_parameters() {
    let config = supported(&["language", "response_format", "extra"]);
    let kept = config.provider_specific_params("m", &given_options(), &[]);
    assert_eq!(kept, object(r#"{"diarize":false,"count":0,"label":""}"#));
}

fn assert_openai_params_are_not_consulted(config: &Supported) {
    let passed = config.provider_specific_params("m", &given_options(), &[String::from("extra")]);
    let ignored = config.provider_specific_params(
        "m",
        &given_options(),
        &[String::from("model"), String::from("language")],
    );
    assert_eq!(passed, ignored);
}

fn assert_reserved_names_are_case_sensitive(config: &Supported) {
    let options = object(r#"{"Model":"m","model":"m","OPENAI_TRANSCRIPTION_PARAMS":"x"}"#);
    let kept = config.provider_specific_params("m", &options, &[]);
    assert_eq!(kept, object(r#"{"Model":"m"}"#));
}

fn assert_options_are_not_mutated(config: &Supported) {
    let options = given_options();
    let before = options.clone();
    config.provider_specific_params("m", &options, &[String::from("extra")]);
    assert_eq!(options, before);
}

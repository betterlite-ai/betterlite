use std::collections::BTreeMap;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_provider::api::transcription_contract::{AudioFile, AudioTranscriptionConfig};
use http::HeaderMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Defaults(DefaultsCase),
    ProviderParameters(ParametersCase),
}

#[derive(Debug, Deserialize)]
pub(crate) struct DefaultsCase {
    data_object: Object,
    data_bytes: String,
    named_files: BTreeMap<String, String>,
    content_type: String,
    api_bases: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ParametersCase {
    model: String,
    supported: Vec<String>,
    supported_with_extra: Vec<String>,
    optional_params: Object,
    reserved_params: Object,
    openai_params: Vec<String>,
    other_openai_params: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
enum HelperFailure {
    #[error("the case bytes are not UTF-8: {source}")]
    NotUtf8 {
        #[source]
        source: std::str::Utf8Error,
    },
}

struct Base;

impl AudioTranscriptionConfig for Base {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }
}

struct Capability {
    native: bool,
    subtitles: bool,
}

impl AudioTranscriptionConfig for Capability {
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

struct Fixture {
    supported: Vec<String>,
}

impl AudioTranscriptionConfig for Fixture {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Defaults(case) => defaults_report(case),
            Input::ProviderParameters(case) => Ok(parameters_report(case)),
        }
    }
}

fn data_view(data: &AudioTranscriptionData) -> HelperResult {
    match data {
        AudioTranscriptionData::Object(object) => Ok(json!({"kind": "object", "value": object})),
        AudioTranscriptionData::Bytes(bytes) => {
            let text = std::str::from_utf8(bytes)
                .map_err(|source| HelperFailure::NotUtf8 { source })
                .or_value_error()?;
            Ok(json!({"kind": "bytes", "value": text}))
        }
    }
}

fn files_view(files: Option<&AudioTranscriptionFiles>) -> serde_json::Value {
    match files {
        None => json!({"kind": "absent"}),
        Some(found) if found.is_empty() => json!({"kind": "empty"}),
        Some(found) => json!({"kind": "named", "value": found}),
    }
}

fn record_view(record: &AudioTranscriptionRequestData) -> HelperResult {
    Ok(json!({
        "data": data_view(&record.data)?,
        "files": files_view(record.files.as_ref()),
        "content_type": record.content_type,
    }))
}

fn named_files(values: &BTreeMap<String, String>) -> AudioTranscriptionFiles {
    values
        .iter()
        .map(|(name, content)| {
            (
                name.clone(),
                AudioTranscriptionFileValue::Text(content.clone()),
            )
        })
        .collect()
}

fn capability_view<C>(config: &C) -> serde_json::Value
where
    C: AudioTranscriptionConfig,
{
    json!({
        "native": config.has_native_endpoint(),
        "subtitles": config.supports_subtitle_synthesis(),
    })
}

fn capability_views() -> serde_json::Value {
    json!({
        "base": capability_view(&Base),
        "native": capability_view(&Capability { native: true, subtitles: false }),
        "subtitle": capability_view(&Capability { native: false, subtitles: true }),
    })
}

fn url_views<C>(base: &C, api_bases: &[Option<String>]) -> Vec<String>
where
    C: AudioTranscriptionConfig,
{
    let empty = Object::new();
    api_bases
        .iter()
        .map(|api_base| base.complete_url(api_base.as_deref(), "m", &empty, &empty))
        .collect()
}

fn diagnostics<C>(base: &C) -> (String, String)
where
    C: AudioTranscriptionConfig,
{
    let empty = Object::new();
    let audio = AudioFile {
        filename: "a.wav",
        content: b"abc",
        content_type: "audio/wav",
    };
    let request = match base.transform_audio_request("m", &audio, &empty, &empty) {
        Ok(_) => String::new(),
        Err(error) => error.to_string(),
    };
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: Vec::new(),
    };
    let reply = match base.transform_audio_response(&response) {
        Ok(_) => String::new(),
        Err(error) => error.to_string(),
    };
    (request, reply)
}

fn request_views(
    case: &DefaultsCase,
) -> Result<serde_json::Map<String, serde_json::Value>, Box<HelperError>> {
    let object_data = AudioTranscriptionData::Object(case.data_object.clone());
    let default_record = record_view(&AudioTranscriptionRequestData {
        data: object_data.clone(),
        files: None,
        content_type: None,
    })?;
    let bytes_record = record_view(&AudioTranscriptionRequestData {
        data: AudioTranscriptionData::Bytes(case.data_bytes.clone().into_bytes()),
        files: None,
        content_type: None,
    })?;
    let explicit_record = record_view(&AudioTranscriptionRequestData {
        data: object_data.clone(),
        files: Some(AudioTranscriptionFiles::new()),
        content_type: Some(case.content_type.clone()),
    })?;
    let named_record = record_view(&AudioTranscriptionRequestData {
        data: object_data,
        files: Some(named_files(&case.named_files)),
        content_type: None,
    })?;
    Ok(serde_json::Map::from_iter([
        (String::from("default_record"), default_record),
        (String::from("bytes_record"), bytes_record),
        (String::from("explicit_record"), explicit_record),
        (String::from("named_record"), named_record),
    ]))
}

fn defaults_report(case: &DefaultsCase) -> HelperResult {
    let base = Base;
    let mut views = request_views(case)?;
    views.insert(String::from("capabilities"), capability_views());
    views.insert(
        String::from("urls"),
        json!(url_views(&base, &case.api_bases)),
    );
    let (request_error, response_error) = diagnostics(&base);
    views.insert(String::from("request_error"), json!(request_error));
    views.insert(String::from("response_error"), json!(response_error));
    Ok(serde_json::Value::Object(views))
}

fn parameters_report(case: &ParametersCase) -> serde_json::Value {
    let config = Fixture {
        supported: case.supported.clone(),
    };
    let filtered =
        config.provider_specific_params(&case.model, &case.optional_params, &case.openai_params);
    let openai_changed = config.provider_specific_params(
        &case.model,
        &case.optional_params,
        &case.other_openai_params,
    );
    let with_extra = Fixture {
        supported: case.supported_with_extra.clone(),
    };
    let supported_extra = with_extra.provider_specific_params(
        &case.model,
        &case.optional_params,
        &case.openai_params,
    );
    let reserved =
        config.provider_specific_params(&case.model, &case.reserved_params, &case.openai_params);
    json!({
        "filtered": filtered,
        "openai_changed": openai_changed,
        "supported_extra": supported_extra,
        "reserved": reserved,
        "after": case.optional_params,
    })
}

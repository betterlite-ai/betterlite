use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::sections::mistral_resources::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, ProcessedAudioFile, TupleContent,
    process_audio_file,
};
use betterlite_openai::mistral_audio::error::MistralAudioError;
use betterlite_openai::mistral_audio::transcription::{
    VoxtralTranscription, complete_url, decode_voxtral_transcript, map_openai_params,
    transcription_request, validate_environment,
};
use betterlite_openai::openai_base_config::Object;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    probes: Vec<Probe>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    model: String,
    filename: String,
    content: String,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    non_default_params: Object,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    environment_api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    reply: Option<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(no_file(path))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn io::Read + Send>> {
        Err(no_file(path))
    }
}

fn no_file(path: &Path) -> io::Error {
    io::Error::other(format!(
        "a tuple of bytes opens no file: {}",
        path.display()
    ))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        input
            .probes
            .iter()
            .map(probe_view)
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}

fn probe_view(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let audio = processed(probe)?;
    let reply = probe.reply.as_ref().map(upstream).transpose()?;
    probe_value(probe, &audio, reply.as_ref()).map_err(failure)
}

fn failure(error: MistralAudioError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("MistralAudioError"),
        status: Some(error.status()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn probe_value(
    probe: &Probe,
    audio: &ProcessedAudioFile,
    reply: Option<&UpstreamResponse>,
) -> Result<Value, MistralAudioError> {
    let settings = Settings {
        api_base: None,
        api_key: probe.environment_api_key.clone().map(SecretString::from),
    };
    let key = probe.api_key.clone().map(SecretString::from);
    let mut mapped = probe.optional_params.clone();
    map_openai_params(&probe.non_default_params, &mut mapped);
    let caller: IndexMap<String, String> = probe
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let headers = validate_environment(&caller, key.as_ref(), &settings);
    let request = transcription_request(&probe.model, audio, &mapped)?;
    let response = match reply {
        None => Value::Null,
        Some(raw) => transcription_view(&decode_voxtral_transcript(raw)?),
    };
    Ok(json!({
        "mapped": mapped,
        "headers": headers,
        "url": complete_url(probe.api_base.as_deref()),
        "request": {
            "form": request.form,
            "filename": request.file.filename,
            "content_type": request.file.content_type,
            "content": BASE64.encode(request.file.file_content),
        },
        "response": response,
    }))
}

fn processed(probe: &Probe) -> Result<ProcessedAudioFile, Box<HelperError>> {
    let tuple = AudioFile::Tuple(AudioTuple {
        filename: &probe.filename,
        content: Some(TupleContent::Bytes(probe.content.as_bytes())),
    });
    process_audio_file(tuple, &NoFiles).or_config_error()
}

fn upstream(reply: &Reply) -> Result<UpstreamResponse, Box<HelperError>> {
    let headers = HeaderMap::try_from(&reply.headers).or_config_error()?;
    Ok(UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body.clone().into_bytes(),
    })
}

fn transcription_view(decoded: &VoxtralTranscription) -> Value {
    let mut fields = Map::new();
    for name in ["segments", "language"] {
        if let Some(value) = decoded.response.extra.get(name) {
            fields.insert(name.to_owned(), value.clone());
        }
    }
    json!({
        "text": decoded.response.text,
        "fields": fields,
        "hidden": decoded.hidden,
    })
}

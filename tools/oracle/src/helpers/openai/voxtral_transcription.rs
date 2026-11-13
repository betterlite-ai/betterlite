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
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Map, Value, json};

use super::voxtral_speech::{Input, Probe, failure, upstream};
use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

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
    probe_value(probe, &audio, reply.as_ref()).map_err(|error| failure(&error))
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
    let caller: IndexMap<String, String> = probe
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let headers = validate_environment(&caller, key.as_ref(), &settings);
    let mut mapped = probe.optional_params.clone();
    map_openai_params(&probe.non_default_params, &mut mapped);
    let request = transcription_request(&probe.model, audio, &mapped)?;
    let response = match reply {
        None => Value::Null,
        Some(raw) => transcription_view(&decode_voxtral_transcript(raw)?),
    };
    Ok(json!({
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

use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::sections::mistral_resources::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::mistral_audio::error::MistralAudioError;
use betterlite_openai::mistral_audio::speech::{
    decode_voxtral_reply, map_openai_params, request_body,
};
use betterlite_openai::mistral_audio::transcription::{
    map_openai_params as map_transcription_params, validate_environment,
};
use betterlite_openai::openai_base_config::Object;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

// Both Voxtral operations share one probe shape; each operation reads the fields its transformation takes
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    probes: Vec<Probe>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    model: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    voice: Option<SpeechVoice>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    non_default_params: Object,
    #[serde(default)]
    kwargs: Object,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    environment_api_key: Option<String>,
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
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        input
            .probes
            .iter()
            .map(speech_view)
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}
pub(crate) struct TranscriptionOp;
impl Helper for TranscriptionOp {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        input
            .probes
            .iter()
            .map(transcription_view)
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}
fn speech_view(probe: &Probe) -> Result<Value, Box<HelperError>> {
    speech_value(
        probe,
        probe.reply.as_ref().map(upstream).transpose()?.as_ref(),
    )
    .map_err(|error| failure(&error))
}
fn speech_value(probe: &Probe, reply: Option<&UpstreamResponse>) -> Result<Value, MistralAudioError> {
    let (voice, params) =
        map_openai_params(&probe.optional_params, probe.voice.as_ref(), &probe.kwargs);
    let response = match reply {
        None => Value::Null,
        Some(raw) => {
            let request = request_body("voxtral", "", voice.as_deref(), &params);
            let content = decode_voxtral_reply(&request, raw)?;
            json!({
                "status": content.status,
                "headers": content.headers,
                "body": BASE64.encode(content.body),
            })
        }
    };
    Ok(json!({
        "mapping": {"voice": voice, "params": params},
        "response": response,
    }))
}
fn transcription_view(probe: &Probe) -> Result<Value, Box<HelperError>> {
    transcription_value(probe).map_err(|error| failure(&error))
}
fn transcription_value(probe: &Probe) -> Result<Value, MistralAudioError> {
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
    map_transcription_params(&probe.non_default_params, &mut mapped);
    Ok(json!({ "headers": headers, "mapped": mapped }))
}
fn upstream(reply: &Reply) -> Result<UpstreamResponse, Box<HelperError>> {
    let headers = HeaderMap::try_from(&reply.headers).or_config_error()?;
    Ok(UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body.clone().into_bytes(),
    })
}
fn failure(error: &MistralAudioError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("MistralAudioError"),
        status: Some(error.status()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

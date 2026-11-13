use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::mistral_audio::error::MistralAudioError;
use betterlite_openai::mistral_audio::speech::{
    decode_voxtral_reply, map_openai_params, request_body,
};
use betterlite_openai::openai_base_config::Object;
use http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

// Both Voxtral operations share one probe shape; each module reads the fields its transformation takes
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) probes: Vec<Probe>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) filename: String,
    #[serde(default)]
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) voice: Option<SpeechVoice>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) non_default_params: Object,
    #[serde(default)]
    pub(crate) kwargs: Object,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment_api_key: Option<String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) reply: Option<Reply>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
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
            .map(probe_view)
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}
fn probe_view(probe: &Probe) -> Result<Value, Box<HelperError>> {
    probe_value(
        probe,
        probe.reply.as_ref().map(upstream).transpose()?.as_ref(),
    )
    .map_err(|error| failure(&error))
}
fn probe_value(
    probe: &Probe,
    reply: Option<&UpstreamResponse>,
) -> Result<Value, MistralAudioError> {
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
pub(crate) fn upstream(reply: &Reply) -> Result<UpstreamResponse, Box<HelperError>> {
    let headers = HeaderMap::try_from(&reply.headers).or_config_error()?;
    Ok(UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body.clone().into_bytes(),
    })
}
pub(crate) fn failure(error: &MistralAudioError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("MistralAudioError"),
        status: Some(error.status()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

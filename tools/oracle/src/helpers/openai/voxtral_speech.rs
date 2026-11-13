use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::sections::mistral_resources::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::mistral_audio::error::MistralAudioError;
use betterlite_openai::mistral_audio::speech::{
    decode_voxtral_reply, map_openai_params, request_body, validate_environment,
};
use betterlite_openai::openai_base_config::Object;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

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
    #[serde(default)]
    voice: Option<SpeechVoice>,
    #[serde(default)]
    optional_params: Object,
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
    .map_err(failure)
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
    reply: Option<&UpstreamResponse>,
) -> Result<Value, MistralAudioError> {
    let settings = Settings {
        api_base: None,
        api_key: probe.environment_api_key.clone().map(SecretString::from),
    };
    let key = probe.api_key.clone().map(SecretString::from);
    let (voice, params) =
        map_openai_params(&probe.optional_params, probe.voice.as_ref(), &probe.kwargs);
    let caller: IndexMap<String, String> = probe
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let headers = validate_environment(&caller, key.as_ref(), &settings)?;
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
        "headers": headers,
        "response": response,
    }))
}

fn upstream(reply: &Reply) -> Result<UpstreamResponse, Box<HelperError>> {
    let headers = HeaderMap::try_from(&reply.headers).or_config_error()?;
    Ok(UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body.clone().into_bytes(),
    })
}

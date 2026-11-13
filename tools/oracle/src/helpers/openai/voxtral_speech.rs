use std::collections::BTreeMap;

use betterlite_config::sections::mistral_resources::Settings;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::mistral_audio::speech::map_openai_params;
use betterlite_openai::mistral_audio::transcription::{
    map_openai_params as map_transcription_params, validate_environment,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

mod tests;

// Both Voxtral operations share one probe shape; each operation reads the fields its transformation takes
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    probes: Vec<Probe>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    model: String,
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
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        Ok(Value::Array(input.probes.iter().map(speech_view).collect()))
    }
}
pub(crate) struct TranscriptionOp;
impl Helper for TranscriptionOp {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        Ok(Value::Array(
            input.probes.iter().map(transcription_view).collect(),
        ))
    }
}
fn speech_view(probe: &Probe) -> Value {
    let (voice, params) =
        map_openai_params(&probe.optional_params, probe.voice.as_ref(), &probe.kwargs);
    json!({ "voice": voice, "params": params })
}
fn transcription_view(probe: &Probe) -> Value {
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
    json!({ "headers": headers, "mapped": mapped })
}

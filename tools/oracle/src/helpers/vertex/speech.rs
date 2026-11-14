use std::future::ready;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use betterlite_vertex::vertex_speech::audio::decode_vertex_speech_audio;
use betterlite_vertex::vertex_speech::mapping::{
    SpeechKwargs, SpeechMapping, SpeechOptionalParams, SpeechVoice, map_openai_params,
};
use betterlite_vertex::vertex_speech::request::{SpeechCall, prepare_request};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod input;
mod tests;

use input::{Case, Input};

struct FixedMint;

impl MintAccessToken for FixedMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        let project = project.unwrap_or("project-1").to_owned();
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from("oracle-token"),
            project,
        })))
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        Ok(match input {
            Input::AlloyVoice { voices } => {
                let views: Vec<Value> = mapped(voices)
                    .iter()
                    .map(|mapping| json!({"voice": mapping.voice, "dict": mapping.voice_dict}))
                    .collect();
                json!({"voices": views})
            }
            Input::CustomVoiceLanguage { voices } => {
                let dicts: Vec<Option<Object>> = mapped(voices)
                    .into_iter()
                    .map(|mapping| mapping.voice_dict)
                    .collect();
                json!({"voices": dicts})
            }
            Input::AacMapsMp3 { formats } => aac_maps_mp3(formats),
            Input::SsmlSubstring { cases } => case_views(&runtime, cases, "inputs", "input"),
            Input::FullAudioConfig { cases } => {
                case_views(&runtime, cases, "configs", "audioConfig")
            }
            Input::VoiceDictPrecedence { cases } => case_views(&runtime, cases, "voices", "voice"),
            Input::DecodedStatus200 { bodies } => decoded(bodies),
        })
    }
}

fn mapped(voices: &[Option<SpeechVoice>]) -> Vec<SpeechMapping> {
    let optional = SpeechOptionalParams::default();
    let kwargs = SpeechKwargs::default();
    voices
        .iter()
        .map(|voice| map_openai_params(voice.as_ref(), &optional, &kwargs))
        .collect()
}

fn aac_maps_mp3(formats: &[Option<String>]) -> Value {
    let encodings: Vec<String> = formats
        .iter()
        .map(|format| {
            let optional = SpeechOptionalParams {
                response_format: format.clone(),
                speed: None,
            };
            map_openai_params(None, &optional, &SpeechKwargs::default()).audio_encoding
        })
        .collect();
    json!({"encodings": encodings})
}

fn case_views(runtime: &Runtime, cases: &[Case], key: &str, field: &str) -> Value {
    let views: Vec<Value> = cases
        .iter()
        .map(|case| {
            let voice = case.voice.clone().map(SpeechVoice::Name);
            let optional = SpeechOptionalParams {
                response_format: case.response_format.clone(),
                speed: case.speed,
            };
            let kwargs = SpeechKwargs {
                audio_config: case.audio_config.clone(),
                use_ssml: case.use_ssml,
            };
            let mut mapping = map_openai_params(voice.as_ref(), &optional, &kwargs);
            if let Some(dict) = &case.mapped_voice {
                mapping.voice_dict = Some(dict.clone());
            }
            let call = SpeechCall {
                input: case.input.as_deref().unwrap_or("hi"),
                voice: mapping.voice.as_deref(),
                mapping: &mapping,
                voice_dict: case.litellm_voice.as_ref(),
                credentials: None,
                project: None,
                location: None,
            };
            match runtime.block_on(prepare_request(&FixedMint, &call, &IndexMap::new())) {
                Ok(request) => json!(request.body).get(field).cloned().unwrap_or_default(),
                Err(_error) => json!({"error": true}),
            }
        })
        .collect();
    json!({key: views})
}

fn decoded(bodies: &[String]) -> Value {
    let sniff = |audio: &[u8]| audio.starts_with(b"fLaC").then(|| "audio/flac".to_owned());
    let views: Vec<Value> = bodies
        .iter()
        .map(|body| match decode_vertex_speech_audio(body.as_bytes(), &sniff) {
            Ok(audio) => json!({"status": audio.status, "contentType": audio.content_type, "bytes": audio.bytes}),
            Err(_error) => json!({"error": true}),
        })
        .collect();
    json!({"bodies": views})
}

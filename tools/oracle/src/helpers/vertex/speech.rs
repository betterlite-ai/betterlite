use std::future::ready;
use std::io::Error as IoError;

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

use crate::helpers::{Helper, HelperError, HelperResult};

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
        let runtime = Builder::new_current_thread().build().map_err(failure)?;
        Ok(match input {
            Input::AlloyVoice { voices } => alloy_voice(voices),
            Input::CustomVoiceLanguage { voices } => custom_voice_language(voices),
            Input::AacMapsMp3 { formats } => aac_maps_mp3(formats),
            Input::SsmlSubstring { cases } => ssml(&runtime, cases),
            Input::FullAudioConfig { cases } => audio(&runtime, cases),
            Input::VoiceDictPrecedence { cases } => voices(&runtime, cases),
            Input::DecodedStatus200 { bodies } => decoded(bodies),
        })
    }
}

fn failure(error: IoError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn voice_mapping(name: &str) -> SpeechMapping {
    let voice = SpeechVoice::Name(name.to_owned());
    map_openai_params(
        Some(&voice),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    )
}

fn alloy_voice(voices: &[Option<SpeechVoice>]) -> Value {
    let views: Vec<Value> = voices
        .iter()
        .map(|voice| {
            let mapping = map_openai_params(
                voice.as_ref(),
                &SpeechOptionalParams::default(),
                &SpeechKwargs::default(),
            );
            json!({"voice": mapping.voice, "dict": mapping.voice_dict})
        })
        .collect();
    json!({"voices": views})
}

fn custom_voice_language(voices: &[String]) -> Value {
    let dicts: Vec<Option<Object>> = voices
        .iter()
        .map(|name| voice_mapping(name).voice_dict)
        .collect();
    json!({"voices": dicts})
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

struct CaseView<'case> {
    mapping: SpeechMapping,
    input: &'case str,
    voice_dict: Option<&'case Object>,
    voice: Option<String>,
}

fn views(runtime: &Runtime, cases: &[CaseView<'_>], wrapper: &str, field: &str) -> Vec<Value> {
    cases
        .iter()
        .map(|case| {
            let call = SpeechCall {
                input: case.input,
                voice: case.voice.as_deref(),
                mapping: &case.mapping,
                voice_dict: case.voice_dict,
                credentials: None,
                project: None,
                location: None,
            };
            match runtime.block_on(prepare_request(&FixedMint, &call, &IndexMap::new())) {
                Ok(request) => project(json!(request.body), wrapper, field),
                Err(_error) => json!({"error": true}),
            }
        })
        .collect()
}

fn project(view: Value, wrapper: &str, field: &str) -> Value {
    let value = view.get(field).cloned().unwrap_or(Value::Null);
    if wrapper.is_empty() {
        return value;
    }
    json!({wrapper: value})
}

fn ssml(runtime: &Runtime, cases: &[Case]) -> Value {
    let mut prepared: Vec<CaseView> = Vec::new();
    for case in cases {
        let kwargs = SpeechKwargs {
            audio_config: None,
            use_ssml: case.use_ssml,
        };
        prepared.push(CaseView {
            mapping: map_openai_params(None, &SpeechOptionalParams::default(), &kwargs),
            input: case.input.as_deref().unwrap_or_default(),
            voice_dict: None,
            voice: None,
        });
    }
    json!({"inputs": views(runtime, &prepared, "input", "input")})
}

fn audio_view(case: &Case) -> CaseView<'_> {
    let optional = SpeechOptionalParams {
        response_format: case.response_format.clone(),
        speed: case.speed,
    };
    let kwargs = SpeechKwargs {
        audio_config: case.audio_config.clone(),
        use_ssml: false,
    };
    CaseView {
        mapping: map_openai_params(None, &optional, &kwargs),
        input: "hi",
        voice_dict: None,
        voice: None,
    }
}

fn audio(runtime: &Runtime, cases: &[Case]) -> Value {
    let prepared: Vec<CaseView> = cases.iter().map(audio_view).collect();
    json!({"configs": views(runtime, &prepared, "", "audioConfig")})
}

fn voices(runtime: &Runtime, cases: &[Case]) -> Value {
    let prepared: Vec<CaseView> = cases.iter().map(voice_view).collect();
    json!({"voices": views(runtime, &prepared, "voice", "voice")})
}

fn view_with(
    voice: Option<&str>,
    mapped: Option<&Object>,
    voice_dict: Option<&Object>,
) -> CaseView<'_> {
    let voice = voice.map(|name| SpeechVoice::Name(name.to_owned()));
    let mut mapping = map_openai_params(
        voice.as_ref(),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    if let Some(dict) = mapped {
        mapping.voice_dict = Some(dict.clone());
    }
    CaseView {
        voice: mapping.voice.clone(),
        mapping,
        input: "hi",
        voice_dict,
    }
}

fn voice_view(case: &Case) -> CaseView<'_> {
    view_with(
        case.voice.as_deref(),
        case.mapped_voice.as_ref(),
        case.litellm_voice.as_ref(),
    )
}

fn decoded(bodies: &[String]) -> Value {
    let sniff = |audio: &[u8]| (audio.first() == Some(&b'X')).then_some("audio/x-test".to_owned());
    let views: Vec<Value> = bodies
        .iter()
        .map(|body| match decode_vertex_speech_audio(body.as_bytes(), &sniff) {
            Ok(audio) => json!({"status": audio.status, "contentType": audio.content_type, "bytes": audio.bytes}),
            Err(_error) => json!({"error": true}),
        })
        .collect();
    json!({"bodies": views})
}

use std::future::ready;
use std::io::Error as IoError;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use betterlite_vertex::vertex_speech::audio::decode_vertex_speech_audio;
use betterlite_vertex::vertex_speech::error::VertexSpeechError;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::vertex_speech::mapping::{
    SpeechAudioConfig, SpeechKwargs, SpeechMapping, SpeechOptionalParams, SpeechVoice,
    map_openai_params,
};
use betterlite_vertex::vertex_speech::request::{SpeechCall, SpeechRequest, prepare_request};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperError, HelperResult};

mod input;
mod tests;

use input::{AudioCase, Input, SsmlCase, VoiceDictCase};

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
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(failure)?;
        match input {
            Input::AlloyVoice { voices } => Ok(alloy_voice(voices)),
            Input::CustomVoiceLanguage { voices } => Ok(custom_voice_language(voices)),
            Input::AacMapsMp3 { formats } => Ok(aac_maps_mp3(formats)),
            Input::SsmlSubstring { cases } => Ok(ssml_substring(&runtime, cases)),
            Input::FullAudioConfig { cases } => Ok(full_audio_config(&runtime, cases)),
            Input::VoiceDictPrecedence { cases } => Ok(voice_dict_precedence(&runtime, cases)),
            Input::DecodedStatus200 { bodies } => Ok(decoded_status_200(bodies)),
        }
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

fn alloy_voice(voices: &[Option<SpeechVoice>]) -> Value {
    let mapped_voices: Vec<Value> = voices
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
    json!({"voices": mapped_voices})
}

fn custom_voice_language(voices: &[String]) -> Value {
    let languages: Vec<Value> = voices
        .iter()
        .map(|name| {
            let voice = SpeechVoice::Name(name.clone());
            let mapping = map_openai_params(
                Some(&voice),
                &SpeechOptionalParams::default(),
                &SpeechKwargs::default(),
            );
            json!(mapping.voice_dict)
        })
        .collect();
    json!({"voices": languages})
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

fn prepared(
    runtime: &Runtime,
    mapping: &SpeechMapping,
    input: &str,
    voice_dict: Option<&Object>,
    voice: Option<&str>,
) -> Result<SpeechRequest, VertexSpeechError> {
    let call = SpeechCall {
        input,
        voice,
        mapping,
        voice_dict,
        credentials: None,
        project: None,
        location: None,
    };
    runtime.block_on(prepare_request(&FixedMint, &call, &IndexMap::new()))
}

// the helper reports only the kind of failure, without comparing an error's class or message
fn error_code(error: &VertexSpeechError) -> &'static str {
    match error {
        VertexSpeechError::ResponseJson(_) => "json",
        VertexSpeechError::ResponseNotObject => "not_object",
        VertexSpeechError::AudioContentNotText => "not_text",
        VertexSpeechError::SsmlRequired => "value_error",
        VertexSpeechError::InputRequired => "value_error",
        VertexSpeechError::BothInputs => "value_error",
        VertexSpeechError::MissingAudioContent => "value_error",
        VertexSpeechError::AudioContentNotAscii => "value_error",
        VertexSpeechError::Base64(_) => "value_error",
        VertexSpeechError::Mint(_) => "other",
        VertexSpeechError::Location(_) => "other",
        VertexSpeechError::ResponseTooLarge { .. } => "other",
        VertexSpeechError::UnknownVoiceField { .. } => "other",
        VertexSpeechError::VoiceFieldType { .. } => "other",
    }
}

fn case_body(
    runtime: &Runtime,
    mapping: &SpeechMapping,
    voice_dict: Option<&Object>,
    voice: Option<&str>,
    input: &str,
) -> Value {
    match prepared(runtime, mapping, input, voice_dict, voice) {
        Ok(request) => json!(request.body),
        Err(error) => json!({"error": error_code(&error)}),
    }
}

fn project(view: Value, wrapper: &str, field: &str) -> Value {
    if view.get("error").is_some() {
        return view;
    }
    if wrapper.is_empty() {
        return view[field].clone();
    }
    json!({wrapper: view[field]})
}

fn ssml_substring(runtime: &Runtime, cases: &[SsmlCase]) -> Value {
    let inputs: Vec<Value> = cases
        .iter()
        .map(|case| {
            let kwargs = SpeechKwargs { audio_config: None, use_ssml: case.use_ssml };
            let mapping = map_openai_params(None, &SpeechOptionalParams::default(), &kwargs);
            project(case_body(runtime, &mapping, None, None, &case.input), "input", "input")
        })
        .collect();
    json!({"inputs": inputs})
}

fn full_audio_config(runtime: &Runtime, cases: &[AudioCase]) -> Value {
    let configs: Vec<Value> = cases
        .iter()
        .map(|case| {
            let optional = SpeechOptionalParams {
                response_format: case.response_format.clone(),
                speed: case.speed,
            };
            let kwargs = SpeechKwargs { audio_config: case.audio_config.clone(), use_ssml: false };
            let mapping = map_openai_params(None, &optional, &kwargs);
            project(case_body(runtime, &mapping, None, None, "hi"), "", "audioConfig")
        })
        .collect();
    json!({"configs": configs})
}

fn voice_dict_precedence(runtime: &Runtime, cases: &[VoiceDictCase]) -> Value {
    let voices: Vec<Value> = cases
        .iter()
        .map(|case| {
            let voice = case.voice.clone().map(SpeechVoice::Name);
            let mut mapping = map_openai_params(
                voice.as_ref(),
                &SpeechOptionalParams::default(),
                &SpeechKwargs::default(),
            );
            if let Some(dict) = &case.mapped_voice {
                mapping.voice_dict = Some(dict.clone());
            }
            let view = case_body(runtime, &mapping, case.litellm_voice.as_ref(), mapping.voice.as_deref(), "hi");
            project(view, "voice", "voice")
        })
        .collect();
    json!({"voices": voices})
}

fn decoded_status_200(bodies: &[String]) -> Value {
    let sniff = |audio: &[u8]| (audio.first() == Some(&b'X')).then_some("audio/x-test".to_owned());
    let decoded: Vec<Value> = bodies
        .iter()
        .map(|body| match decode_vertex_speech_audio(body.as_bytes(), &sniff) {
            Ok(audio) => json!({
                "status": audio.status,
                "contentType": audio.content_type,
                "bytes": audio.bytes,
            }),
            Err(error) => json!({"error": error_code(&error)}),
        })
        .collect();
    json!({"bodies": decoded})
}

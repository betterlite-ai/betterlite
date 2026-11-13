use std::future::ready;
use std::io::Error as IoError;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use betterlite_vertex::vertex_speech::audio::decode_vertex_speech_audio;
use betterlite_vertex::vertex_speech::error::VertexSpeechError;
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
        let runtime = Builder::new_current_thread().build().map_err(failure)?;
        Ok(match input {
            Input::AlloyVoice { voices } => alloy_voice(voices),
            Input::CustomVoiceLanguage { voices } => custom_voice_language(voices),
            Input::AacMapsMp3 { formats } => aac_maps_mp3(formats),
            Input::SsmlSubstring { cases } => ssml_substring(&runtime, cases),
            Input::FullAudioConfig { cases } => full_audio_config(&runtime, cases),
            Input::VoiceDictPrecedence { cases } => voice_dict_precedence(&runtime, cases),
            Input::DecodedStatus200 { bodies } => decoded_status_200(bodies),
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

struct PreparedCase<'case> {
    mapping: SpeechMapping,
    input: &'case str,
    voice_dict: Option<&'case Object>,
    voice: Option<String>,
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

fn mapped_voice(name: &str) -> Option<Object> {
    let voice = SpeechVoice::Name(name.to_owned());
    map_openai_params(
        Some(&voice),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    )
    .voice_dict
}

fn custom_voice_language(voices: &[String]) -> Value {
    json!({"voices": voices.iter().map(|name| mapped_voice(name)).collect::<Vec<_>>()})
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

fn views(runtime: &Runtime, prepared: &[PreparedCase<'_>], wrapper: &str, field: &str) -> Vec<Value> {
    prepared
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
                Err(error) => json!({"error": error_code(&error)}),
            }
        })
        .collect()
}

fn project(view: Value, wrapper: &str, field: &str) -> Value {
    if view.get("error").is_some() {
        return view;
    }
    let value = view.get(field).cloned().unwrap_or(Value::Null);
    if wrapper.is_empty() {
        return value;
    }
    json!({wrapper: value})
}

fn ssml_case(case: &SsmlCase) -> PreparedCase<'_> {
    let kwargs = SpeechKwargs {
        audio_config: None,
        use_ssml: case.use_ssml,
    };
    PreparedCase {
        mapping: map_openai_params(None, &SpeechOptionalParams::default(), &kwargs),
        input: &case.input,
        voice_dict: None,
        voice: None,
    }
}

fn audio_case(case: &AudioCase) -> PreparedCase<'_> {
    let optional = SpeechOptionalParams {
        response_format: case.response_format.clone(),
        speed: case.speed,
    };
    let kwargs = SpeechKwargs {
        audio_config: case.audio_config.clone(),
        use_ssml: false,
    };
    PreparedCase {
        mapping: map_openai_params(None, &optional, &kwargs),
        input: "hi",
        voice_dict: None,
        voice: None,
    }
}

fn voice_case(case: &VoiceDictCase) -> PreparedCase<'_> {
    let voice = case.voice.clone().map(SpeechVoice::Name);
    let mut mapping = map_openai_params(
        voice.as_ref(),
        &SpeechOptionalParams::default(),
        &SpeechKwargs::default(),
    );
    if let Some(dict) = &case.mapped_voice {
        mapping.voice_dict = Some(dict.clone());
    }
    PreparedCase {
        voice: mapping.voice.clone(),
        mapping,
        input: "hi",
        voice_dict: case.litellm_voice.as_ref(),
    }
}

fn ssml_substring(runtime: &Runtime, cases: &[SsmlCase]) -> Value {
    json!({"inputs": views(runtime, &cases.iter().map(ssml_case).collect::<Vec<_>>(), "input", "input")})
}

fn full_audio_config(runtime: &Runtime, cases: &[AudioCase]) -> Value {
    json!({"configs": views(runtime, &cases.iter().map(audio_case).collect::<Vec<_>>(), "", "audioConfig")})
}

fn voice_dict_precedence(runtime: &Runtime, cases: &[VoiceDictCase]) -> Value {
    json!({"voices": views(runtime, &cases.iter().map(voice_case).collect::<Vec<_>>(), "voice", "voice")})
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

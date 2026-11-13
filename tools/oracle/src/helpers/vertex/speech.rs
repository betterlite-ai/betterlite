use std::future::ready;
use std::io::Error as IoError;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use betterlite_vertex::vertex_speech::audio::decode_vertex_speech_audio;
use betterlite_vertex::vertex_speech::error::VertexSpeechError;
use betterlite_vertex::vertex_speech::mapping::{
    SpeechAudioConfig, SpeechKwargs, SpeechMapping, SpeechOptionalParams, SpeechVoice,
    map_openai_params,
};
use betterlite_vertex::vertex_speech::request::{SpeechCall, SpeechRequest, prepare_request};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    AlloyVoice {
        voices: Vec<Option<SpeechVoice>>,
    },
    CustomVoiceLanguage {
        voices: Vec<String>,
    },
    AacMapsMp3 {
        formats: Vec<Option<String>>,
    },
    SsmlSubstring {
        cases: Vec<SsmlCase>,
    },
    FullAudioConfig {
        cases: Vec<AudioCase>,
    },
    VoiceDictPrecedence {
        cases: Vec<VoiceDictCase>,
    },
    DecodedStatus200 {
        bodies: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct SsmlCase {
    pub(crate) input: String,
    #[serde(default)]
    pub(crate) use_ssml: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AudioCase {
    #[serde(default)]
    pub(crate) response_format: Option<String>,
    #[serde(default)]
    pub(crate) speed: Option<i64>,
    #[serde(default)]
    pub(crate) audio_config: Option<SpeechAudioConfig>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VoiceDictCase {
    #[serde(default)]
    pub(crate) litellm_voice: Option<Object>,
    #[serde(default)]
    pub(crate) mapped_voice: Option<Object>,
    #[serde(default)]
    pub(crate) voice: Option<String>,
}

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

fn input_error_code(error: &VertexSpeechError) -> &'static str {
    match error {
        VertexSpeechError::SsmlRequired => "ssml_required",
        VertexSpeechError::InputRequired => "no_input",
        VertexSpeechError::BothInputs => "both_inputs",
        VertexSpeechError::Mint(_) => "other",
        VertexSpeechError::Location(_) => "other",
        VertexSpeechError::ResponseTooLarge { .. } => "other",
        VertexSpeechError::ResponseJson(_) => "other",
        VertexSpeechError::ResponseNotObject => "other",
        VertexSpeechError::MissingAudioContent => "other",
        VertexSpeechError::AudioContentNotText => "other",
        VertexSpeechError::AudioContentNotAscii => "other",
        VertexSpeechError::Base64(_) => "other",
        VertexSpeechError::UnknownVoiceField { .. } => "other",
        VertexSpeechError::VoiceFieldType { .. } => "other",
    }
}

// the response path reports only the kind of failure, the same way the Python helper normalizes it
fn response_error_code(error: &VertexSpeechError) -> &'static str {
    match error {
        VertexSpeechError::ResponseJson(_) => "json",
        VertexSpeechError::ResponseNotObject => "not_object",
        VertexSpeechError::AudioContentNotText => "not_text",
        VertexSpeechError::MissingAudioContent => "value_error",
        VertexSpeechError::AudioContentNotAscii => "value_error",
        VertexSpeechError::Base64(_) => "value_error",
        VertexSpeechError::Mint(_) => "other",
        VertexSpeechError::Location(_) => "other",
        VertexSpeechError::ResponseTooLarge { .. } => "other",
        VertexSpeechError::SsmlRequired => "other",
        VertexSpeechError::InputRequired => "other",
        VertexSpeechError::BothInputs => "other",
        VertexSpeechError::UnknownVoiceField { .. } => "other",
        VertexSpeechError::VoiceFieldType { .. } => "other",
    }
}

fn ssml_substring(runtime: &Runtime, cases: &[SsmlCase]) -> Value {
    let inputs: Vec<Value> = cases
        .iter()
        .map(|case| {
            let kwargs = SpeechKwargs {
                audio_config: None,
                use_ssml: case.use_ssml,
            };
            let mapping = map_openai_params(None, &SpeechOptionalParams::default(), &kwargs);
            match prepared(runtime, &mapping, &case.input, None, None) {
                Ok(request) => json!({"input": request.body.input}),
                Err(error) => json!({"error": input_error_code(&error)}),
            }
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
            let kwargs = SpeechKwargs {
                audio_config: case.audio_config.clone(),
                use_ssml: false,
            };
            let mapping = map_openai_params(None, &optional, &kwargs);
            match prepared(runtime, &mapping, "hi", None, None) {
                Ok(request) => json!(request.body.audio_config),
                Err(error) => json!({"error": input_error_code(&error)}),
            }
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
            match prepared(
                runtime,
                &mapping,
                "hi",
                case.litellm_voice.as_ref(),
                mapping.voice.as_deref(),
            ) {
                Ok(request) => json!(request.body.voice),
                Err(error) => json!({"error": input_error_code(&error)}),
            }
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
            Err(error) => json!({"error": response_error_code(&error)}),
        })
        .collect();
    json!({"bodies": decoded})
}

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use betterlite_config::sections::Sections;
use betterlite_core::api::deepgram::transcription::request::{
    DeepgramTranscriptionConfig, SUPPORTED_OPENAI_PARAMS, map_openai_params, raw_audio_request,
};
use betterlite_core::api::deepgram::transcription::response::decode_deepgram_transcript;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, TupleContent,
};
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::Object;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    env: BTreeMap<String, String>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Params {
        optional: Object,
        non_default: Object,
    },
    Request {
        audio: AudioKind,
        filename: String,
        content: String,
    },
    Url {
        model: String,
        api_base: Option<String>,
        options: Object,
    },
    Auth {
        api_key: Option<String>,
    },
    Reply {
        body: String,
    },
    ErrorClass {
        message: String,
        status_code: u16,
        headers: BTreeMap<String, String>,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AudioKind {
    Bytes,
    Tuple,
}

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }
}

fn step(step: &Step, config: &DeepgramTranscriptionConfig) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Params {
            optional,
            non_default,
        } => Ok(json!({
            "supported": SUPPORTED_OPENAI_PARAMS,
            "mapped": map_openai_params(non_default, optional.clone()),
        })),
        Step::Request {
            audio,
            filename,
            content,
        } => {
            let bytes = content.as_bytes();
            let file = match audio {
                AudioKind::Bytes => AudioFile::Bytes(bytes),
                AudioKind::Tuple => AudioFile::Tuple(AudioTuple {
                    filename,
                    content: Some(TupleContent::Bytes(bytes)),
                }),
            };
            let Ok(request) = raw_audio_request(file, &NoFiles) else {
                return Ok(json!({ "raised": "ValueError" }));
            };
            let AudioTranscriptionData::Bytes(data) = request.data else {
                return Err("the request data is not raw bytes").or_config_error();
            };
            Ok(json!({
                "data": data,
                "files": request.files,
                "content_type": request.content_type,
            }))
        }
        Step::Url {
            model,
            api_base,
            options,
        } => Ok(json!(config.complete_url(
            api_base.as_deref(),
            model,
            options
        ))),
        Step::Auth { api_key } => {
            let key = api_key.as_deref().map(SecretString::from);
            Ok(json!(config.validate_environment(key.as_ref())))
        }
        Step::ErrorClass {
            message,
            status_code,
            headers,
        } => {
            let error =
                DeepgramTranscriptionConfig::get_error_class(message, *status_code, headers);
            Ok(json!({
                "status_code": error.status_code,
                "message": error.message,
                "headers": error.headers,
            }))
        }
        Step::Reply { body } => {
            let Ok(transcript) = decode_deepgram_transcript(body.as_bytes()) else {
                return Ok(json!({ "raised": "ValueError" }));
            };
            let mut response = serde_json::to_value(&transcript).or_config_error()?;
            let hidden = response
                .as_object_mut()
                .and_then(|fields| fields.remove("hidden_params"));
            Ok(json!({ "response": response, "hidden": hidden }))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let (api_key, api_base) = Sections::deepgram_transcript_settings(&input.env);
        let config = DeepgramTranscriptionConfig { api_key, api_base };
        let answers = input
            .steps
            .iter()
            .map(|one| step(one, &config))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(answers))
    }
}

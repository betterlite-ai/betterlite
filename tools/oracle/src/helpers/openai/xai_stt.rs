use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, TupleContent, process_audio_file,
};
use betterlite_openai::xai_stt::decode_xai_stt_reply;
use betterlite_openai::xai_stt::{
    FormValue, SUPPORTED_OPENAI_PARAMS, authorization_headers, map_openai_params, stt_url,
    transcription_form,
};
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::xai_files::{LIMITS, authorization, outcome, settings, upstream};
use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    env: BTreeMap<String, String>,
    xai_key: Option<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Params {
        optional: Map<String, Value>,
        non_default: Map<String, Value>,
    },
    Url {
        api_base: Option<String>,
    },
    Auth {
        api_key: Option<String>,
    },
    Form(FormStep),
    Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FormStep {
    model: String,
    filename: Option<String>,
    content: String,
    extra_body: IndexMap<String, FormValue>,
    options: IndexMap<String, FormValue>,
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

fn form_step(step: &FormStep) -> Result<Value, Box<HelperError>> {
    let bytes = step.content.as_bytes();
    let file = step
        .filename
        .as_deref()
        .map_or(AudioFile::Bytes(bytes), |name| {
            AudioFile::Tuple(AudioTuple {
                filename: name,
                content: Some(TupleContent::Bytes(bytes)),
            })
        });
    let audio = process_audio_file(file, &NoFiles).or_config_error()?;
    let form = transcription_form(&step.model, &audio, &step.extra_body, &step.options);
    let fields: Vec<Value> = form
        .fields
        .iter()
        .map(|(name, value)| json!([name, value.form_text()]))
        .collect();
    let part = form.file;
    Ok(json!({ "form": fields, "file": [part.file_name, part.content_type, part.data] }))
}

fn step(step: &Step, settings: &Settings) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Params {
            optional,
            non_default,
        } => {
            let pairs: Vec<(String, Value)> = non_default.clone().into_iter().collect();
            let mut mapped = optional.clone();
            map_openai_params(&pairs, &mut mapped);
            Ok(json!({ "supported": SUPPORTED_OPENAI_PARAMS, "mapped": mapped }))
        }
        Step::Url { api_base } => Ok(json!(stt_url(api_base.as_deref(), settings))),
        Step::Auth { api_key } => {
            let key = api_key.as_deref().map(SecretString::from);
            let headers = authorization_headers(HeaderMap::new(), key.as_ref(), settings);
            authorization(headers.map_err(|error| error.status_code()))
        }
        Step::Form(form) => form_step(form),
        Step::Reply {
            status,
            headers,
            body,
        } => {
            let reply = decode_xai_stt_reply(&upstream(*status, body, headers)?, &LIMITS);
            outcome(reply.map_err(|error| error.status_code()))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let settings = settings(input.xai_key.as_deref(), &input.env);
        let answers = input
            .steps
            .iter()
            .map(|one| step(one, &settings))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(answers))
    }
}

mod tests;

mod tests;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Object;
use betterlite_vertex::studio_interactions::credentials::{RequestHeaders, StudioParams};
use betterlite_vertex::studio_transcription::decode::decode_studio_transcription;
use betterlite_vertex::studio_transcription::error::StudioTranscriptionError;
use betterlite_vertex::studio_transcription::request::{
    map_openai_params, transform_audio_request, validate_environment,
};
use betterlite_vertex::studio_transcription::{SUPPORTED_PARAMS, SUPPORTS_SUBTITLE_SYNTHESIS};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Supported,
    Subtitle,
    Params {
        optional: Object,
        non_default: Object,
    },
    Environment {
        headers: Vec<(String, String)>,
        api_key: Option<String>,
    },
    Request {
        model: String,
        content: String,
        optional: Object,
    },
    Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
}

// a raised step keeps only LiteLLM's status, since helper answers compare no error class or message
fn outcome<T>(result: Result<T, StudioTranscriptionError>) -> Result<Value, Box<HelperError>>
where
    T: Serialize,
{
    match result {
        Ok(value) => serde_json::to_value(value).or_config_error(),
        Err(error) => Ok(json!({ "raised": error.status_code() })),
    }
}

// a secret has no Serialize impl, so the headers leave as the plain text the wire carries
fn rendered_headers(headers: &RequestHeaders) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

fn step(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Supported => serde_json::to_value(SUPPORTED_PARAMS).or_config_error(),
        Step::Subtitle => Ok(Value::Bool(SUPPORTS_SUBTITLE_SYNTHESIS)),
        Step::Params {
            optional,
            non_default,
        } => serde_json::to_value(map_openai_params(non_default, optional)).or_config_error(),
        Step::Environment { headers, api_key } => {
            let caller: IndexMap<String, String> = headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            let params = StudioParams {
                api_key: api_key.as_deref().map(SecretString::from),
            };
            let checked = validate_environment(&caller, &params, &Settings::default());
            outcome(checked.map(|headers| rendered_headers(&headers)))
        }
        Step::Request {
            model,
            content,
            optional,
        } => {
            let audio = ProcessedAudioFile {
                file_content: content.as_bytes().to_vec(),
                filename: "audio.wav".to_owned(),
                content_type: "audio/wav".to_owned(),
            };
            let data = transform_audio_request(model, &audio, optional).or_config_error()?;
            serde_json::to_value(data.data).or_config_error()
        }
        Step::Reply {
            status,
            headers,
            body,
        } => {
            let mut map = HeaderMap::new();
            for (name, value) in headers {
                map.insert(
                    HeaderName::try_from(name.as_str()).or_config_error()?,
                    HeaderValue::try_from(value.as_str()).or_config_error()?,
                );
            }
            let reply = UpstreamResponse {
                status: *status,
                headers: map,
                body: body.as_bytes().to_vec(),
            };
            outcome(decode_studio_transcription(&reply))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let answers: Result<Vec<_>, _> = input.steps.iter().map(step).collect();
        Ok(Value::Array(answers?))
    }
}

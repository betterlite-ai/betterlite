mod tests;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_vertex::vertex_recognition::error::VertexRecognitionError;
use betterlite_vertex::vertex_recognition::request::{
    complete_url, map_openai_params, recognize_request,
};
use betterlite_vertex::vertex_recognition::response::decode_vertex_recognition;
use http::header::{HeaderMap, HeaderName, HeaderValue};
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
    Params {
        optional: Object,
        non_default: Object,
        drop_params: bool,
        global_drop_params: bool,
    },
    Url {
        api_base: Option<String>,
        location: Option<String>,
        project: Option<String>,
    },
    Request {
        model: String,
        content: String,
        language: Option<Json>,
    },
    Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
}

fn upstream(
    status: u16,
    body: &str,
    headers: &[(String, String)],
) -> Result<UpstreamResponse, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            HeaderName::try_from(name.as_str()).or_config_error()?,
            HeaderValue::try_from(value.as_str()).or_config_error()?,
        );
    }
    Ok(UpstreamResponse {
        status,
        headers: map,
        body: body.as_bytes().to_vec(),
    })
}

// a raised step keeps only LiteLLM's status, since helper answers compare no error class or message
fn outcome<T>(result: Result<T, VertexRecognitionError>) -> Result<Value, Box<HelperError>>
where
    T: Serialize,
{
    match result {
        Ok(value) => serde_json::to_value(value).or_config_error(),
        Err(error) => Ok(json!({ "raised": error.status_code() })),
    }
}

fn request_step(
    model: &str,
    content: &str,
    language: Option<&Json>,
) -> Result<Value, Box<HelperError>> {
    let audio = ProcessedAudioFile {
        file_content: content.as_bytes().to_vec(),
        filename: "audio.wav".to_owned(),
        content_type: "audio/wav".to_owned(),
    };
    serde_json::to_value(recognize_request(model, &audio, language)).or_config_error()
}

fn step(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Params {
            optional,
            non_default,
            drop_params,
            global_drop_params,
        } => outcome(map_openai_params(
            non_default,
            optional,
            DropParams {
                global: *global_drop_params,
                request: *drop_params,
            },
        )),
        Step::Url {
            api_base,
            location,
            project,
        } => outcome(complete_url(
            api_base.as_deref(),
            location.as_deref(),
            project.as_deref(),
            None,
        )),
        Step::Request {
            model,
            content,
            language,
        } => request_step(model, content, language.as_ref()),
        Step::Reply {
            status,
            headers,
            body,
        } => {
            let reply = upstream(*status, body, headers)?;
            outcome(decode_vertex_recognition(&reply))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let answers = input
            .steps
            .iter()
            .map(step)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(answers))
    }
}

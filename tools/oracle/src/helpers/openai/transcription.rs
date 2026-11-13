mod exchange;
mod tests;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::transcription::config::{
    TranscriptionFamily, complete_url, map_openai_params, supported_openai_params,
    transform_response, validate_environment,
};
use betterlite_openai::transcription::duration::{duration_number, srt_vtt_duration};
use betterlite_openai::transcription::error::TranscriptionError;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

const URL_BASES: [Option<&str>; 6] = [
    None,
    Some(""),
    Some("https://speech.test/v1"),
    Some("https://speech.test/v1/"),
    Some("https://speech.test/v2"),
    Some("https://speech.test"),
];

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Exchange,
    Params,
    Duration,
    Auth,
    Response,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Provider {
    Whisper,
    Gpt,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            status: 200,
            headers: BTreeMap::from([(
                String::from("content-type"),
                String::from("application/json"),
            )]),
            body: String::from(r#"{"text":"hi"}"#),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    body: String,
    content_type: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Probe {
    model: String,
    provider: Option<Provider>,
    optional_params: IndexMap<String, Json>,
    non_default_params: IndexMap<String, Json>,
    existing_params: IndexMap<String, Json>,
    headers: BTreeMap<String, String>,
    api_key: Option<String>,
    environment_api_key: Option<String>,
    api_base: Option<String>,
    atranscription: bool,
    reply: Reply,
    data_model: Option<String>,
    data_model_null: bool,
    texts: Vec<String>,
    bodies: Vec<Body>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    probes: Vec<Probe>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut values = Vec::new();
        for probe in &input.probes {
            values.push(match input.function {
                Function::Exchange => exchange::exchange(probe)?,
                Function::Params => params(probe),
                Function::Duration => durations(probe)?,
                Function::Auth => auth(probe),
                Function::Response => responses(probe),
            });
        }
        Ok(Value::Array(values))
    }
}

fn params(probe: &Probe) -> Value {
    let mut fields = serde_json::Map::new();
    for (label, family) in [
        ("whisper", TranscriptionFamily::Whisper),
        ("gpt", TranscriptionFamily::Gpt),
    ] {
        let mut mapped = probe.existing_params.clone();
        mapped.extend(map_openai_params(family, &probe.non_default_params));
        fields.insert(
            format!("{label}_supported"),
            json!(supported_openai_params(family)),
        );
        fields.insert(format!("{label}_mapped"), json!(mapped));
    }
    Value::Object(fields)
}

fn durations(probe: &Probe) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut found = Vec::new();
    for text in &probe.texts {
        let seconds = duration_number(srt_vtt_duration(text)).or_config_error()?;
        found.push(serde_json::to_value(seconds).or_config_error()?);
    }
    Ok(json!({ "durations": found }))
}

fn auth(probe: &Probe) -> Value {
    let urls: Vec<String> = URL_BASES.iter().map(|base| complete_url(*base)).collect();
    let key = |value: &Option<String>| value.clone().map(SecretString::from);
    let caller = probe.headers.clone().into_iter().collect();
    let headers = validate_environment(
        &caller,
        key(&probe.api_key).as_ref(),
        key(&probe.environment_api_key).as_ref(),
    );
    json!({ "urls": urls, "headers": headers })
}

fn responses(probe: &Probe) -> Value {
    let mut results = Vec::new();
    for body in &probe.bodies {
        results.push(
            match transform_response(body.body.as_bytes(), Some(&body.content_type)) {
                Ok(response) => {
                    json!({ "ok": true, "response": exchange::response_view(&response) })
                }
                Err(error) => {
                    let kind = if matches!(error, TranscriptionError::Json { .. }) {
                        "json"
                    } else {
                        "value"
                    };
                    json!({ "ok": false, "kind": kind })
                }
            },
        );
    }
    json!({ "results": results })
}

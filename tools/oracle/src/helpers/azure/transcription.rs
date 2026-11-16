mod exchange;
mod tests;

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_azure::transcription::native::reply::decode_native_transcription;
use betterlite_azure::transcription::native::{
    NativeError, SUPPORTED_OPENAI_PARAMS, complete_url, get_error_class, map_openai_params,
    transform_request, validate_environment,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{AudioFile, AuthorizedAudioFiles};
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::Headers;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Exchange,
    NativeConfig,
    NativeResponse,
    NativeRequest,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Body {
    status: u16,
    body: String,
}

impl Default for Body {
    fn default() -> Self {
        Self {
            status: 200,
            body: String::new(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Probe {
    model: String,
    optional_params: IndexMap<String, Json>,
    non_default_params: IndexMap<String, Json>,
    existing_params: IndexMap<String, Json>,
    headers: BTreeMap<String, String>,
    api_key: Option<String>,
    environment_api_key: Option<String>,
    api_base: Option<String>,
    environment_api_base: Option<String>,
    bases: Vec<Option<String>>,
    atranscription: bool,
    api_version: Option<String>,
    azure_ad_token: Option<String>,
    max_retries: Option<i64>,
    client_kind: Option<String>,
    reply: Reply,
    bodies: Vec<Body>,
    audio: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    probes: Vec<Probe>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut values = Vec::new();
        for probe in &input.probes {
            values.push(match input.function {
                Function::Exchange => exchange::exchange(probe)?,
                Function::NativeConfig => native_config(probe),
                Function::NativeResponse => native_response(probe),
                Function::NativeRequest => native_request(probe)?,
            });
        }
        Ok(Value::Array(values))
    }
}

fn error_view(error: &NativeError) -> Value {
    json!({
        "ok": false,
        "status": error.status(),
    })
}

fn native_config(probe: &Probe) -> Value {
    let key = probe.api_key.clone().map(SecretString::from);
    let environment_key = probe.environment_api_key.clone().map(SecretString::from);
    let caller: Headers = probe.headers.clone().into_iter().collect();
    let headers = match validate_environment(&caller, key.as_ref(), environment_key.as_ref()) {
        Ok(headers) => json!({ "ok": true, "value": headers }),
        Err(error) => error_view(&error),
    };
    let mut urls = Vec::new();
    for base in &probe.bases {
        urls.push(
            match complete_url(
                base.as_deref(),
                probe.environment_api_base.as_deref(),
                &probe.optional_params,
            ) {
                Ok(url) => json!({ "ok": true, "url": url }),
                Err(error) => error_view(&error),
            },
        );
    }
    let failure = get_error_class(String::from("boom"), 502, Headers::new());
    json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": map_openai_params(&probe.non_default_params, probe.existing_params.clone()),
        "headers": headers,
        "urls": urls,
        "error_status": failure.status(),
    })
}

fn native_response(probe: &Probe) -> Value {
    let headers: Headers = probe.headers.clone().into_iter().collect();
    let mut results = Vec::new();
    for body in &probe.bodies {
        results.push(
            match decode_native_transcription(body.status, &headers, body.body.as_bytes()) {
                Ok(reply) => json!({
                    "ok": true,
                    "text": reply.response.text,
                    "hidden": reply.hidden_params,
                }),
                Err(error) => error_view(&error),
            },
        );
    }
    json!({ "results": results })
}

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no authorized files",
        ))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no authorized files",
        ))
    }
}

fn native_request(probe: &Probe) -> HelperResult {
    let request =
        transform_request(AudioFile::Bytes(probe.audio.as_bytes()), &NoFiles).or_config_error()?;
    let AudioTranscriptionData::Bytes(bytes) = request.data else {
        return Err::<Value, String>(String::from("the native request data is not bytes"))
            .or_config_error();
    };
    Ok(json!({
        "kind": "bytes",
        "body": BASE64.encode(bytes),
        "content_type": request.content_type,
    }))
}

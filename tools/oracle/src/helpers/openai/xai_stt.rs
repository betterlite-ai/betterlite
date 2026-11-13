use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, ProcessedAudioFile, TupleContent,
    process_audio_file,
};
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::xai_stt::error::XaiSttError;
use betterlite_openai::xai_stt::reply::decode_xai_stt_reply;
use betterlite_openai::xai_stt::request::{
    FormText, FormValue, SUPPORTED_OPENAI_PARAMS, SttForm, authorization_headers,
    map_openai_params, stt_url, transcription_form,
};
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Request {
        #[serde(default)]
        env: BTreeMap<String, String>,
        xai_key: Option<String>,
        calls: Vec<RequestCall>,
    },
    Reply {
        #[serde(default, rename = "env")]
        _env: BTreeMap<String, String>,
        #[serde(rename = "xai_key")]
        _xai_key: Option<String>,
        calls: Vec<ReplyCall>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AudioSpec {
    filename: Option<String>,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    api_base: Option<String>,
    api_key: Option<String>,
    model: String,
    audio: AudioSpec,
    extra_body: BTreeMap<String, FormValue>,
    options: BTreeMap<String, FormValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
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

fn settings(xai_key: Option<&str>, env: &BTreeMap<String, String>) -> Settings {
    Settings {
        xai_key: xai_key.map(SecretString::from),
        generic_api_key: None,
        environment_key: env
            .get("XAI_API_KEY")
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.as_str())),
        api_base: env
            .get("XAI_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned(),
    }
}

const fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 1 << 20,
        max_event_bytes: 1 << 10,
        max_items: 1000,
    }
}

fn invalid(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failure(error: &XaiSttError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("XaiSttError"),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn upstream(status: u16, body: &[u8], headers: &[(String, String)]) -> UpstreamResponse {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let Ok(header_name) = HeaderName::try_from(name.as_str()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::try_from(value.as_str()) else {
            continue;
        };
        map.insert(header_name, header_value);
    }
    UpstreamResponse {
        status,
        headers: map,
        body: body.to_vec(),
    }
}

fn processed(spec: &AudioSpec) -> Result<ProcessedAudioFile, Box<crate::helpers::HelperError>> {
    let content = spec.content.as_bytes();
    let file = spec.filename.as_deref().map_or_else(
        || AudioFile::Bytes(content),
        |name| {
            AudioFile::Tuple(AudioTuple {
                filename: name,
                content: Some(TupleContent::Bytes(content)),
            })
        },
    );
    process_audio_file(file, &NoFiles).map_err(|error| invalid(error.to_string()))
}

fn fields_value(form: &SttForm) -> Value {
    let fields = form
        .fields
        .iter()
        .map(|(name, value)| match value.form_text() {
            FormText::Scalar(text) => json!([name, text]),
            FormText::Repeated(items) => json!([name, items]),
        })
        .collect();
    Value::Array(fields)
}

fn file_value(form: &SttForm) -> Value {
    json!({
        "filename": form.file.file_name.clone(),
        "content_type": form.file.content_type.clone(),
        "content": form.file.data.clone(),
    })
}

fn mapped_fixture() -> Result<Map<String, Value>, Box<crate::helpers::HelperError>> {
    let mut optional: Map<String, Value> = serde_json::from_str(
        r#"{"response_format":"json","language":"fr"}"#,
    )
    .map_err(|error| invalid(error.to_string()))?;
    map_openai_params(
        &[
            ("language".to_owned(), json!("en")),
            ("prompt".to_owned(), json!("direct")),
        ],
        &mut optional,
    );
    Ok(optional)
}

fn request_call(
    call: &RequestCall,
    settings: &Settings,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let key = call.api_key.as_deref().map(SecretString::from);
    let optional = mapped_fixture()?;
    let mut answer: Map<String, Value> = Map::from_iter([
        (
            "supported".to_owned(),
            json!(SUPPORTED_OPENAI_PARAMS),
        ),
        ("mapped".to_owned(), Value::Object(optional)),
        (
            "url".to_owned(),
            Value::String(stt_url(call.api_base.as_deref(), settings)),
        ),
        ("authorization".to_owned(), Value::Null),
        ("raised".to_owned(), Value::Null),
        ("form".to_owned(), Value::Array(Vec::new())),
        ("file".to_owned(), Value::Null),
    ]);
    let headers = match authorization_headers(HeaderMap::new(), key.as_ref(), settings) {
        Ok(headers) => headers,
        Err(XaiSttError::MissingApiKey) => {
            answer.insert("raised".to_owned(), json!("key"));
            return Ok(Value::Object(answer));
        }
        Err(error) => return Err(failure(&error)),
    };
    answer.insert(
        "authorization".to_owned(),
        json!(headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok())),
    );
    let audio = processed(&call.audio)?;
    let extra_body: IndexMap<String, FormValue> = call.extra_body.clone().into_iter().collect();
    let options: IndexMap<String, FormValue> = call.options.clone().into_iter().collect();
    let form = transcription_form(&call.model, &audio, &extra_body, &options);
    answer.insert("form".to_owned(), fields_value(&form));
    answer.insert("file".to_owned(), file_value(&form));
    Ok(Value::Object(answer))
}

fn reply_call(call: &ReplyCall) -> Result<Value, Box<crate::helpers::HelperError>> {
    let response = upstream(call.status, call.body.as_bytes(), &call.headers);
    match decode_xai_stt_reply(&response, &limits()) {
        Ok(result) => {
            serde_json::to_value(result).map_err(|error| invalid(error.to_string()))
        }
        Err(error) => Ok(json!({"error": {"status": error.status_code()}})),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Request {
                env,
                xai_key,
                calls,
            } => {
                let settings = settings(xai_key.as_deref(), env);
                let answers = calls
                    .iter()
                    .map(|call| request_call(call, &settings))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"calls": answers}))
            }
            Input::Reply { calls, .. } => {
                let answers = calls
                    .iter()
                    .map(reply_call)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"calls": answers}))
            }
        }
    }
}

mod tests;

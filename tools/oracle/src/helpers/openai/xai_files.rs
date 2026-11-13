use std::collections::BTreeMap;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::xai_batches::transformation::get_xai_auth_headers;
use betterlite_openai::xai_files::error::XaiFilesError;
use betterlite_openai::xai_files::reply::{
    decode_file_deleted, decode_file_list, decode_file_object, decode_next_page,
};
use betterlite_openai::xai_files::request::{
    FilesTarget, SUPPORTED_OPENAI_PARAMS, files_url, map_openai_params,
};
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Upload {
        #[serde(default)]
        env: BTreeMap<String, String>,
        xai_key: Option<String>,
        calls: Vec<UploadCall>,
    },
    Files {
        #[serde(default)]
        env: BTreeMap<String, String>,
        xai_key: Option<String>,
        calls: Vec<FileCall>,
    },
    List {
        #[serde(default)]
        env: BTreeMap<String, String>,
        xai_key: Option<String>,
        calls: Vec<ListCall>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    status: u16,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadCall {
    api_base: Option<String>,
    api_key: Option<String>,
    now: i64,
    reply: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentSpec {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileCall {
    params_api_base: Option<String>,
    file_id: Option<String>,
    delete_reply: Reply,
    content_reply: ContentSpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListCall {
    params_api_base: Option<String>,
    #[serde(rename = "purpose")]
    _purpose: Option<String>,
    now: i64,
    reply: Reply,
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

fn failure(error: &XaiFilesError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("XaiFilesError"),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn encoded<T>(value: &T) -> Result<Value, Box<crate::helpers::HelperError>>
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|error| invalid(error.to_string()))
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

fn file_value(reply: &Reply, now: i64) -> Result<Value, Box<crate::helpers::HelperError>> {
    let response = upstream(reply.status, reply.body.as_bytes(), &[]);
    match decode_file_object(&response, &limits(), now) {
        Ok(file) => encoded(&file),
        Err(error) if error.status_code().is_some() => Err(failure(&error)),
        Err(_) => Ok(json!({"raised": "reply"})),
    }
}

fn deleted_value(reply: &Reply) -> Result<Value, Box<crate::helpers::HelperError>> {
    let response = upstream(reply.status, reply.body.as_bytes(), &[]);
    match decode_file_deleted(&response, &limits()) {
        Ok(deleted) => encoded(&deleted),
        Err(error) if error.status_code().is_some() => Err(failure(&error)),
        Err(_) => Ok(json!({"raised": "reply"})),
    }
}

fn upload_call(
    call: &UploadCall,
    settings: &Settings,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let key = call.api_key.as_deref().map(SecretString::from);
    let headers = get_xai_auth_headers(HeaderMap::new(), key.as_ref(), settings)
        .map_err(|error| failure(&XaiFilesError::from(error)))?;
    let url = files_url(call.api_base.as_deref(), FilesTarget::Collection, settings)
        .map_err(|error| failure(&error))?;
    let parsed: Map<String, Value> = serde_json::from_str(
        r#"{"purpose":"batch","expires_after":{"anchor":"created_at"}}"#,
    )
    .map_err(|error| invalid(error.to_string()))?;
    let mapped = map_openai_params(
        parsed,
        &[
            ("purpose".to_owned(), json!("assistants")),
            ("temperature".to_owned(), json!(2_i32)),
        ],
        false,
    );
    Ok(json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": encoded(&mapped)?,
        "url": url,
        "authorization": headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()),
        "file": file_value(&call.reply, call.now)?,
    }))
}

fn target(
    call: &FileCall,
    content: bool,
    settings: &Settings,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let file_id = call.file_id.as_deref().unwrap_or("");
    let target = if content {
        FilesTarget::Content(file_id)
    } else {
        FilesTarget::File(file_id)
    };
    match files_url(call.params_api_base.as_deref(), target, settings) {
        Ok(url) => Ok(json!({"url": url})),
        Err(XaiFilesError::Path { .. }) => Ok(json!({"raised": "path"})),
        Err(error) => Err(failure(&error)),
    }
}

fn content_call(spec: &ContentSpec) -> Value {
    let content = BinaryContent::from_response(upstream(spec.status, &spec.body, &spec.headers));
    let watched: Map<String, Value> = content
        .headers
        .into_iter()
        .map(|(name, value)| (name, Value::String(value)))
        .collect();
    json!({
        "status": content.status,
        "headers": Value::Object(watched),
        "body": content.body,
    })
}

fn file_call(
    call: &FileCall,
    settings: &Settings,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    Ok(json!({
        "delete": target(call, false, settings)?,
        "content": target(call, true, settings)?,
        "deleted": deleted_value(&call.delete_reply)?,
        "content_reply": content_call(&call.content_reply),
    }))
}

fn list_call(
    call: &ListCall,
    settings: &Settings,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let url = files_url(
        call.params_api_base.as_deref(),
        FilesTarget::Collection,
        settings,
    )
    .map_err(|error| failure(&error))?;
    let response = upstream(call.reply.status, call.reply.body.as_bytes(), &[]);
    let mut entry: Map<String, Value> = Map::from_iter([
        ("url".to_owned(), Value::String(url.clone())),
        ("next".to_owned(), Value::Null),
        ("raised".to_owned(), Value::Null),
        ("files".to_owned(), Value::Array(Vec::new())),
        ("raised_list".to_owned(), Value::Null),
    ]);
    match decode_next_page(&response, &limits(), &url) {
        Ok(Some(page)) => {
            entry.insert(
                "next".to_owned(),
                json!({"url": page.url, "query": page.query}),
            );
        }
        Ok(None) => {}
        Err(_) => {
            entry.insert("raised".to_owned(), json!("page"));
        }
    }
    match decode_file_list(&response, &limits(), call.now) {
        Ok(files) => {
            let values = files.iter().map(encoded).collect::<Result<Vec<_>, _>>()?;
            entry.insert("files".to_owned(), Value::Array(values));
        }
        Err(error) if error.status_code().is_some() => return Err(failure(&error)),
        Err(_) => {
            entry.insert("raised_list".to_owned(), json!("list"));
        }
    }
    Ok(Value::Object(entry))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Upload {
                env,
                xai_key,
                calls,
            } => {
                let settings = settings(xai_key.as_deref(), env);
                let answers = calls
                    .iter()
                    .map(|call| upload_call(call, &settings))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"calls": answers}))
            }
            Input::Files {
                env,
                xai_key,
                calls,
            } => {
                let settings = settings(xai_key.as_deref(), env);
                let answers = calls
                    .iter()
                    .map(|call| file_call(call, &settings))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"calls": answers}))
            }
            Input::List {
                env,
                xai_key,
                calls,
            } => {
                let settings = settings(xai_key.as_deref(), env);
                let answers = calls
                    .iter()
                    .map(|call| list_call(call, &settings))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"calls": answers}))
            }
        }
    }
}

mod tests;

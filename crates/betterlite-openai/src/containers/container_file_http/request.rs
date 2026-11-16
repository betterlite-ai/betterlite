use std::collections::BTreeMap;

use betterlite_http::url_path::encode_segment;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_models::decisions::Json;
use betterlite_prompts::media_metadata::extract_file_data;
use indexmap::IndexMap;
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value};
use url::Url;

use super::transport::{ContainerMethod, UploadedFile};
use super::{ContainerFileHttpError, ContainerFileUpload};
use crate::api::container_contracts::config::BaseContainerConfig;

const DEFAULT_FILENAME: &str = "file";

// A path or query value as the caller supplied it; query text is the value itself or its decimal digits
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamValue {
    Text(String),
    Integer(i64),
}

impl ParamValue {
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Integer(number) => number.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SslVerify {
    Enabled(bool),
    CaBundle(String),
}

// The litellm_params fields this handler reads
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallParams<'params> {
    pub api_key: Option<&'params str>,
    pub api_base: Option<&'params str>,
    pub ssl_verify: Option<SslVerify>,
}

fn text_param<'params>(
    params: &'params Map<String, Value>,
    field: &'static str,
) -> Result<Option<&'params str>, ContainerFileHttpError> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(ContainerFileHttpError::ParamType {
            field,
            expected: "a string",
        }),
    }
}

pub fn read_params(params: &Map<String, Value>) -> Result<CallParams<'_>, ContainerFileHttpError> {
    let ssl_verify = match params.get("ssl_verify") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(enabled)) => Some(SslVerify::Enabled(*enabled)),
        Some(Value::String(bundle)) => Some(SslVerify::CaBundle(bundle.clone())),
        Some(_) => {
            return Err(ContainerFileHttpError::ParamType {
                field: "ssl_verify",
                expected: "a boolean or a CA bundle path",
            });
        }
    };
    Ok(CallParams {
        api_key: text_param(params, "api_key")?,
        api_base: text_param(params, "api_base")?,
        ssl_verify,
    })
}

// Extra headers go to validate_environment first and are laid over its answer after, so they win
pub fn request_headers(
    config: &dyn BaseContainerConfig,
    extra_headers: Option<&BTreeMap<String, String>>,
    api_key: Option<&str>,
) -> BTreeMap<String, String> {
    let none = BTreeMap::new();
    let extra = extra_headers.unwrap_or(&none);
    let mut headers = config.validate_environment(extra, api_key);
    headers.extend(
        extra
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    headers
}

// The base already ends in /containers and may carry a query, so the template loses its prefix and the query stays
pub fn build_url(
    api_base: &str,
    path_template: &str,
    path_values: &[(&str, String)],
) -> Result<String, ContainerFileHttpError> {
    let mut path = path_template
        .strip_prefix("/containers")
        .unwrap_or(path_template)
        .to_owned();
    for (name, value) in path_values {
        let encoded = encode_segment(value, name)?;
        path = path.replace(&format!("{{{name}}}"), &encoded);
    }
    let mut base = Url::parse(api_base).map_err(|source| ContainerFileHttpError::BaseUrl {
        url: api_base.to_owned(),
        source,
    })?;
    let current = percent_decode_str(base.path()).decode_utf8_lossy();
    let new_path = format!("{}{path}", current.trim_end_matches('/'));
    base.set_path(&new_path);
    Ok(base.to_string())
}

// Declared names in catalog order, absent ones skipped, then the overlay replacing or appending
pub fn build_query(
    names: &[String],
    values: &BTreeMap<String, ParamValue>,
    overlay: Option<&IndexMap<String, ParamValue>>,
) -> IndexMap<String, String> {
    let mut query = IndexMap::new();
    for name in names {
        if let Some(value) = values.get(name) {
            query.insert(name.clone(), value.render());
        }
    }
    for (name, value) in overlay.into_iter().flatten() {
        query.insert(name.clone(), value.render());
    }
    query
}

pub fn pre_call_record(
    url: &str,
    headers: &BTreeMap<String, String>,
    query: &IndexMap<String, String>,
) -> PreCall {
    let logged_headers = headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    let logged_query = query
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(url.to_owned()),
            headers: Some(logged_headers),
            complete_input_dict: None,
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::from([("params".to_owned(), Json::Object(logged_query))]),
        },
    }
}

pub fn parse_method(method: &str) -> Result<ContainerMethod, ContainerFileHttpError> {
    let upper = method.to_ascii_uppercase();
    match upper.as_str() {
        "GET" => Ok(ContainerMethod::Get),
        "DELETE" => Ok(ContainerMethod::Delete),
        "POST" => Ok(ContainerMethod::Post),
        _ => Err(ContainerFileHttpError::UnsupportedMethod { method: upper }),
    }
}

// httpx sets the multipart content type itself, so both spellings LiteLLM removes leave the headers
pub fn multipart_upload(
    upload: &ContainerFileUpload,
    headers: &BTreeMap<String, String>,
) -> Result<(UploadedFile, BTreeMap<String, String>), ContainerFileHttpError> {
    let extracted = extract_file_data(&upload.file_data)?;
    let file = UploadedFile {
        filename: extracted
            .filename
            .unwrap_or_else(|| DEFAULT_FILENAME.to_owned()),
        content: extracted.content,
        content_type: extracted.content_type,
    };
    let mut kept = headers.clone();
    kept.remove("content-type");
    kept.remove("Content-Type");
    Ok((file, kept))
}

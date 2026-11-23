use std::time::Duration;

use betterlite_core::api::skills::get_delete::call::{DEFAULT_SKILLS_PROVIDER, SkillsCallParams};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;
use betterlite_sdk::call_files::{CallFile, files_from_payload};
use betterlite_sdk::call_setup::RequestMap;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;

mod tests;

pub(super) mod create;
pub(super) mod list;
pub(super) mod skill;

const FILES_KEY: &str = "files";
const FILE_KEY: &str = "$file";

// A skill call argument outside the type LiteLLM declares: docs/compatibility.md answers it with a 400 naming the field
#[derive(Debug, thiserror::Error)]
pub(super) enum RequestError {
    #[error("the {name} field does not have the declared type")]
    FieldType {
        provider: String,
        name: &'static str,
    },
}

impl RequestError {
    fn field(provider: &str, name: &'static str) -> Self {
        Self::FieldType {
            provider: provider.to_owned(),
            name,
        }
    }

    pub(super) fn class(&self) -> ErrorClass {
        let Self::FieldType { provider, name } = self;
        ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
            ErrorContext {
                message: self.to_string(),
                model: None,
                provider: Some(provider.clone()),
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: Some((*name).to_owned()),
                body_code: None,
            },
            None,
        )))
    }
}

pub(super) struct SkillFile {
    pub(super) name: String,
    pub(super) content_type: Option<String>,
    pub(super) bytes: Vec<u8>,
}

pub(super) struct SkillArguments {
    pub(super) params: SkillsCallParams,
    pub(super) skill_id: String,
    pub(super) kwargs: IndexMap<String, Json>,
    pub(super) litellm_call_id: Option<String>,
}

pub(super) struct CreateArguments {
    pub(super) params: SkillsCallParams,
    pub(super) display_title: Option<String>,
    pub(super) files: Option<Vec<Json>>,
    pub(super) extra_body: Option<IndexMap<String, Json>>,
    pub(super) kwargs: IndexMap<String, Json>,
    pub(super) litellm_call_id: Option<String>,
}

pub(super) struct ListArguments {
    pub(super) params: SkillsCallParams,
    pub(super) limit: Option<i64>,
    pub(super) page: Option<String>,
    pub(super) source: Option<String>,
    pub(super) kwargs: IndexMap<String, Json>,
    pub(super) litellm_call_id: Option<String>,
}

// The Json shape a skill argument must have; absent and null both take LiteLLM's default instead.
pub(super) trait Field: Sized {
    fn take(json: Json) -> Option<Self>;
}

impl Field for String {
    fn take(json: Json) -> Option<Self> {
        let Json::Text(text) = json else {
            return None;
        };
        Some(text)
    }
}

impl Field for IndexMap<String, Json> {
    fn take(json: Json) -> Option<Self> {
        let Json::Object(fields) = json else {
            return None;
        };
        Some(fields)
    }
}

impl Field for f64 {
    fn take(json: Json) -> Option<Self> {
        match json {
            Json::Number(number) => number.as_f64(),
            Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

fn field<T>(
    provider: &str,
    value: Option<Json>,
    name: &'static str,
) -> Result<Option<T>, RequestError>
where
    T: Field,
{
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(json) => T::take(json)
            .map(Some)
            .ok_or_else(|| RequestError::field(provider, name)),
    }
}

// Each file Python sent is a $file marker: the bytes stay in the call's blobs and the marker names one.
pub(super) fn files(
    provider: &str,
    items: &[Json],
    blobs: &mut [Vec<u8>],
) -> Result<Vec<SkillFile>, RequestError> {
    for item in items {
        match item {
            Json::Object(fields) if fields.contains_key(FILE_KEY) => {}
            Json::Object(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_) => return Err(RequestError::field(provider, "files")),
        }
    }
    let mut payload = RequestMap::new();
    payload.insert(FILES_KEY.to_owned(), Json::List(items.to_vec()));
    let found = files_from_payload(&payload, blobs)
        .map_err(|_error| RequestError::field(provider, "files"))?;
    Ok(found
        .into_iter()
        .map(|(_key, file): (String, CallFile)| SkillFile {
            name: file.name,
            content_type: file.content_type,
            bytes: file.bytes,
        })
        .collect())
}

// LiteLLM defaults an absent provider to anthropic; a provider of the wrong type is a field error.
fn provider(arguments: &mut IndexMap<String, Json>) -> Result<String, RequestError> {
    match arguments.shift_remove("custom_llm_provider") {
        None | Some(Json::Null) => Ok(DEFAULT_SKILLS_PROVIDER.to_owned()),
        Some(Json::Text(name)) => Ok(name),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => Err(
            RequestError::field(DEFAULT_SKILLS_PROVIDER, "custom_llm_provider"),
        ),
    }
}

// api_key and api_base stay in kwargs, since LiteLLM reads them there as well as from the call's own arguments
fn common(
    provider: &str,
    arguments: &mut IndexMap<String, Json>,
) -> Result<(SkillsCallParams, Option<String>), RequestError> {
    let api_key = field::<String>(provider, arguments.get("api_key").cloned(), "api_key")?
        .map(SecretString::from);
    let api_base = field::<String>(provider, arguments.get("api_base").cloned(), "api_base")?;
    let extra_headers = headers(provider, arguments.shift_remove("extra_headers"))?;
    let extra_query = field::<IndexMap<String, Json>>(
        provider,
        arguments.shift_remove("extra_query"),
        "extra_query",
    )?;
    let timeout = timeout(provider, arguments.shift_remove("timeout"))?;
    let call_id = arguments.get("litellm_call_id").cloned();
    let litellm_call_id = field::<String>(provider, call_id, "litellm_call_id")?;
    let params = SkillsCallParams {
        api_key,
        api_base,
        extra_headers,
        extra_query,
        timeout,
        custom_llm_provider: Some(provider.to_owned()),
    };
    Ok((params, litellm_call_id))
}

fn headers(provider: &str, value: Option<Json>) -> Result<HeaderMap, RequestError> {
    let Some(fields) = field::<IndexMap<String, Json>>(provider, value, "extra_headers")? else {
        return Ok(HeaderMap::new());
    };
    let mut headers = HeaderMap::new();
    for (name, value) in fields {
        let header = HeaderName::try_from(name.as_str())
            .map_err(|_invalid| RequestError::field(provider, "extra_headers"))?;
        let Json::Text(text) = value else {
            return Err(RequestError::field(provider, "extra_headers"));
        };
        let header_value = HeaderValue::from_str(&text)
            .map_err(|_invalid| RequestError::field(provider, "extra_headers"))?;
        headers.append(header, header_value);
    }
    Ok(headers)
}

// LiteLLM's timeout argument is a number of seconds; zero takes REQUEST_TIMEOUT further down.
fn timeout(provider: &str, value: Option<Json>) -> Result<Option<Duration>, RequestError> {
    let Some(seconds) = field::<f64>(provider, value, "timeout")? else {
        return Ok(None);
    };
    Ok(Some(Duration::try_from_secs_f64(seconds).map_err(
        |_invalid| RequestError::field(provider, "timeout"),
    )?))
}

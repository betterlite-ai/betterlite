use std::collections::BTreeMap;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::xai_batches::transformation::get_xai_auth_headers;
use betterlite_openai::xai_files::{FilesTarget, files_url, next_page};
use betterlite_openai::xai_files::{XaiFile, XaiFileDeleted, XaiFileList, decode_xai_upload_reply};
use betterlite_openai::xai_files::{
    decode_file_reply, to_file_deleted, to_file_list, to_file_object,
};
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

pub(super) const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 1 << 20,
    max_event_bytes: 1 << 10,
    max_items: 1000,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    env: BTreeMap<String, String>,
    xai_key: Option<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Target {
    Collection,
    File,
    Content,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Parse {
    File,
    Deleted,
    Next,
    List,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Auth {
        api_key: Option<String>,
    },
    Url(UrlStep),
    Reply(ReplyStep),
    Content {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    api_base: Option<String>,
    target: Target,
    file_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyStep {
    parse: Parse,
    api_base: Option<String>,
    now: i64,
    status: u16,
    body: String,
}

pub(super) fn settings(xai_key: Option<&str>, env: &BTreeMap<String, String>) -> Settings {
    let read = |name: &str| env.get(name).filter(|value| !value.is_empty()).cloned();
    Settings {
        xai_key: xai_key.map(SecretString::from),
        generic_api_key: None,
        environment_key: read("XAI_API_KEY").map(SecretString::from),
        api_base: read("XAI_API_BASE"),
    }
}

pub(super) fn upstream(
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

// LiteLLM's ValueError carries no status and its BaseLLMException carries one, so a raised step keeps only that
pub(super) fn outcome<T>(result: Result<T, Option<u16>>) -> Result<Value, Box<HelperError>>
where
    T: Serialize,
{
    match result {
        Ok(value) => serde_json::to_value(value).or_config_error(),
        Err(status) => Ok(json!({ "raised": status })),
    }
}

pub(super) fn authorization(
    headers: Result<HeaderMap, Option<u16>>,
) -> Result<Value, Box<HelperError>> {
    outcome(headers.map(|map| {
        map.get(AUTHORIZATION)
            .map(HeaderValue::to_str)
            .and_then(Result::ok)
            .map(str::to_owned)
    }))
}

fn reply_step(step: &ReplyStep, settings: &Settings) -> Result<Value, Box<HelperError>> {
    let url =
        files_url(step.api_base.as_deref(), FilesTarget::Collection, settings).or_config_error()?;
    let reply = &upstream(step.status, &step.body, &[])?;
    let now = step.now;
    let result = match step.parse {
        Parse::File => decode_file_reply(reply, &LIMITS, |file: XaiFile| {
            json!(to_file_object(&file, now))
        }),
        Parse::Deleted => decode_file_reply(reply, &LIMITS, |deleted: XaiFileDeleted| {
            json!(to_file_deleted(&deleted))
        }),
        Parse::Next => decode_xai_upload_reply::<XaiFileList>(&reply.body, &LIMITS)
            .map(|page| json!(next_page(&page, &url).map(|next| (next.url, next.query)))),
        Parse::List => decode_file_reply(reply, &LIMITS, |page: XaiFileList| {
            json!(to_file_list(&page, now))
        }),
    };
    outcome(result.map_err(|error| error.status_code()))
}

fn step(step: &Step, settings: &Settings) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Auth { api_key } => {
            let key = api_key.as_deref().map(SecretString::from);
            let headers = get_xai_auth_headers(HeaderMap::new(), key.as_ref(), settings);
            authorization(headers.map_err(|error| error.status_code()))
        }
        Step::Url(url) => {
            let id = url.file_id.as_deref().unwrap_or("");
            let files_target = match url.target {
                Target::Collection => FilesTarget::Collection,
                Target::File => FilesTarget::File(id),
                Target::Content => FilesTarget::Content(id),
            };
            let resolved = files_url(url.api_base.as_deref(), files_target, settings);
            outcome(resolved.map_err(|error| error.status_code()))
        }
        Step::Reply(reply) => reply_step(reply, settings),
        Step::Content {
            status,
            headers,
            body,
        } => {
            let content = BinaryContent::from_response(upstream(*status, body, headers)?);
            Ok(
                json!({ "status": content.status, "headers": content.headers, "body": content.body }),
            )
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

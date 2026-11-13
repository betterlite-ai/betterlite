use std::collections::BTreeMap;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_openai::xai_batches::transformation::get_xai_auth_headers;
use betterlite_openai::xai_files::error::XaiFilesError;
use betterlite_openai::xai_files::records::{XaiFile, XaiFileDeleted};
use betterlite_openai::xai_files::reply::{decode_file_reply, to_file_deleted, to_file_object};
use betterlite_openai::xai_files::request::{
    FilesTarget, SUPPORTED_OPENAI_PARAMS, files_url, map_openai_params,
};
use http::header::{AUTHORIZATION, HeaderMap};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::OrHelperError as _;

use views::{content_call, failure, limits, upstream, value};

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

fn upload_call(
    call: &UploadCall,
    settings: &Settings,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let key = call.api_key.as_deref().map(SecretString::from);
    let headers = get_xai_auth_headers(HeaderMap::new(), key.as_ref(), settings)
        .map_err(|error| failure(&XaiFilesError::from(error)))?;
    let url = files_url(call.api_base.as_deref(), FilesTarget::Collection, settings)
        .map_err(|error| failure(&error))?;
    let parsed: Map<String, Value> =
        serde_json::from_str(r#"{"purpose":"batch","expires_after":{"anchor":"created_at"}}"#)
            .or_config_error()?;
    let mapped = map_openai_params(
        parsed,
        &[
            ("purpose".to_owned(), json!("assistants")),
            ("temperature".to_owned(), json!(2_i32)),
        ],
        false,
    );
    let response = upstream(call.reply.status, call.reply.body.as_bytes(), &[]);
    let file = value(decode_file_reply(&response, &limits(), |file: XaiFile| {
        to_file_object(&file, call.now)
    }))?;
    Ok(json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": serde_json::to_value(&mapped).or_config_error()?,
        "url": url,
        "authorization": headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()),
        "file": file,
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

fn file_call(
    call: &FileCall,
    settings: &Settings,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let delete = target(call, false, settings)?;
    let content = target(call, true, settings)?;
    let response = upstream(
        call.delete_reply.status,
        call.delete_reply.body.as_bytes(),
        &[],
    );
    let deleted = value(decode_file_reply(
        &response,
        &limits(),
        |deleted: XaiFileDeleted| to_file_deleted(&deleted),
    ))?;
    Ok(json!({
        "delete": delete,
        "content": content,
        "deleted": deleted,
        "content_reply": content_call(&call.content_reply),
    }))
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
                    .map(|call| views::list_entry(call, &settings))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({"calls": answers}))
            }
        }
    }
}

mod tests;
mod views;

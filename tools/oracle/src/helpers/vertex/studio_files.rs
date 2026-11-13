use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_openai::api::file_type_records::{TwoStepFileUploadConfig, UploadData};
use betterlite_openai::files::records::{FileObject, FileStatus};
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::studio_files::RequestParams;
use betterlite_vertex::studio_files::metadata::{MetadataShape, decode_studio_file_metadata};
use betterlite_vertex::studio_files::plan::create_file_plan;
use betterlite_vertex::studio_files::urls::{FileTarget, decode_delete_response, target};
use jiff::tz::TimeZone;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    CreatePlan {
        now: i64,
        file: FileInput,
    },
    Replies {
        shape: MetadataShape,
        bodies: Vec<String>,
        timezone: String,
    },
    Targets {
        #[serde(default)]
        delete: bool,
        file_ids: Vec<String>,
        api_key: Option<String>,
        api_base: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    DeleteReply {
        replies: Vec<DeleteReply>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileInput {
    data: Vec<u8>,
    filename: Option<String>,
    content_type: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteReply {
    status: u16,
    url: Option<String>,
    #[serde(default)]
    body: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let calls = input
            .calls
            .iter()
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call) -> HelperResult {
    match call {
        Call::CreatePlan { now, file } => plan_view(*now, file),
        Call::Replies {
            shape,
            bodies,
            timezone,
        } => reply_views(bodies, timezone, *shape),
        Call::Targets {
            delete,
            file_ids,
            api_key,
            api_base,
            environment,
        } => target_views(
            file_ids,
            api_key.as_deref(),
            api_base.as_deref(),
            environment,
            *delete,
        ),
        Call::DeleteReply { replies } => delete_views(replies),
    }
}

fn plan_view(now: i64, file: &FileInput) -> HelperResult {
    let supplied = FileData::Tuple4 {
        filename: file.filename.clone(),
        content: FileBody::Bytes(file.data.clone()),
        content_type: file.content_type.clone(),
        headers: file.headers.clone(),
    };
    let plan = create_file_plan(Some(&supplied), "", now).or_value_error()?;
    let (text, bytes) = plan_parts(&plan).or_value_error()?;
    let initial: Value = serde_json::from_str(text).or_value_error()?;
    Ok(json!({
        "headers": plan.initial_request.headers,
        "data": initial,
        "upload_headers": plan.upload_request.headers,
        "upload_data": bytes,
    }))
}

const fn plan_parts(plan: &TwoStepFileUploadConfig) -> Result<(&str, &[u8]), &'static str> {
    let UploadData::Text(text) = &plan.initial_request.data else {
        return Err("the start request does not carry metadata text");
    };
    let UploadData::Bytes(bytes) = &plan.upload_request.data else {
        return Err("the upload request does not carry bytes");
    };
    Ok((text.as_str(), bytes.as_slice()))
}

fn reply_views(bodies: &[String], timezone: &str, shape: MetadataShape) -> HelperResult {
    let zone = TimeZone::get(timezone).or_value_error()?;
    let context = LocalTimeContext { zone };
    let mut files = Vec::with_capacity(bodies.len());
    for body in bodies {
        let file =
            decode_studio_file_metadata(body.as_bytes(), shape, &context).or_value_error()?;
        files.push(file_view(&file));
    }
    Ok(json!({ "files": files }))
}

fn target_views(
    file_ids: &[String],
    api_key: Option<&str>,
    api_base: Option<&str>,
    environment: &BTreeMap<String, String>,
    delete: bool,
) -> HelperResult {
    let settings = settings(environment);
    let params = RequestParams {
        api_key: api_key.map(SecretString::from),
        api_base: api_base.map(str::to_owned),
    };
    let mut targets = Vec::with_capacity(file_ids.len());
    for file_id in file_ids {
        let found = target(file_id, &params, &settings, !delete).or_value_error()?;
        targets.push(target_view(&found));
    }
    Ok(json!({ "targets": targets }))
}

fn delete_views(replies: &[DeleteReply]) -> HelperResult {
    let mut deleted = Vec::with_capacity(replies.len());
    for reply in replies {
        let result =
            decode_delete_response(reply.status, reply.url.as_deref(), reply.body.as_bytes())
                .or_value_error()?;
        deleted.push(json!({
            "id": result.id,
            "object": "file",
            "deleted": result.deleted,
        }));
    }
    Ok(json!({ "replies": deleted }))
}

fn settings(environment: &BTreeMap<String, String>) -> Settings {
    Settings {
        environment_key: non_empty(environment, "GOOGLE_API_KEY")
            .or_else(|| non_empty(environment, "GEMINI_API_KEY"))
            .map(SecretString::from),
        generic_api_key: None,
        api_base: non_empty(environment, "GEMINI_API_BASE")
            .map_or_else(|| DEFAULT_API_BASE.to_owned(), str::to_owned),
    }
}

fn non_empty<'env>(environment: &'env BTreeMap<String, String>, name: &str) -> Option<&'env str> {
    environment
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

fn file_view(file: &FileObject) -> Value {
    json!({
        "id": file.id,
        "bytes": file.bytes,
        "created_at": file.created_at,
        "filename": file.filename,
        "object": "file",
        "purpose": "user_data",
        "status": file.status.map(status_name),
        "status_details": file.status_details,
    })
}

const fn status_name(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Uploaded => "uploaded",
        FileStatus::Processed => "processed",
        FileStatus::Error => "error",
        FileStatus::Pending => "pending",
    }
}

fn target_view(target: &FileTarget) -> Value {
    let query: Vec<[&str; 2]> = target
        .query
        .iter()
        .map(|(name, value)| [name.as_str(), value.as_str()])
        .collect();
    json!({ "url": target.url, "query": query })
}

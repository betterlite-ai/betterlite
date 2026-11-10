use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::files::SUPPORTED_OPENAI_PARAMS;
use betterlite_anthropic::files::api::{response_error, status_failure};
use betterlite_anthropic::files::error::FilesError;
use betterlite_anthropic::files::request::{
    FilesUrl, create_file_parts, files_url, purpose_query, validate_environment,
};
use betterlite_anthropic::files::wire::{
    decode_anthropic_delete, decode_anthropic_file, decode_anthropic_file_list,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::files::records::{BinaryContent, FileObject};
use betterlite_prompts::media_metadata::{FileBody, FileData};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::common::{failure, header_map, raised, settings};
use super::input::{
    BetaHeaderCase, BetaScenario, ContentCase, CreatedAtCase, DeleteCase, ErrorFixture,
    FailureFixture, FileObjectCase, ListCase, UploadCase, UploadItem,
};
use super::views::{deleted_view, file_view, headers_view, pairs_object, parts_view};
use crate::helpers::{HelperError, HelperResult};

#[derive(Clone, Copy)]
enum UrlKind {
    File,
    Content,
}

pub(crate) fn file_upload_multipart(case: &UploadCase) -> HelperResult {
    let failure = case
        .failure
        .as_ref()
        .map(failure_value)
        .transpose()?
        .unwrap_or(Value::Null);
    let mut uploads = Vec::new();
    for item in &case.uploads {
        uploads.push(upload_view(item, case.now));
    }
    Ok(json!({"supported": SUPPORTED_OPENAI_PARAMS, "uploads": uploads, "failure": failure}))
}

fn upload_view(item: &UploadItem, now_epoch: i64) -> Value {
    match create_file_parts(
        upload_file(item).as_ref(),
        item.purpose.as_deref(),
        now_epoch,
    ) {
        Ok(parts) => json!({"parts": parts_view(&parts)}),
        Err(error) => json!({"raised": raised(&error)}),
    }
}

fn upload_file(item: &UploadItem) -> Option<FileData> {
    if item.missing {
        return None;
    }
    let data = item.data.clone().into_bytes();
    match (&item.filename, &item.content_type) {
        (Some(filename), Some(content_type)) => Some(FileData::Tuple3 {
            filename: Some(filename.clone()),
            content: FileBody::Bytes(data),
            content_type: Some(content_type.clone()),
        }),
        (Some(filename), None) => Some(FileData::Tuple2 {
            filename: Some(filename.clone()),
            content: FileBody::Bytes(data),
        }),
        (None, _) => Some(FileData::Direct(FileBody::Bytes(data))),
    }
}

pub(crate) fn native_file_object(case: &FileObjectCase) -> HelperResult {
    let create = decode_values(&case.bodies, case.now, file_view)?;
    let retrieve = decode_values(&case.bodies, case.now, file_view)?;
    Ok(json!({"create": create, "retrieve": retrieve}))
}

pub(crate) fn created_at_timezone(case: &CreatedAtCase) -> HelperResult {
    let bodies: Vec<Value> = case
        .values
        .iter()
        .map(|value| json!({"created_at": value}))
        .collect();
    let stamps = decode_values(&bodies, case.now, |file| json!(file.created_at))?;
    Ok(json!({"created_at": stamps}))
}

fn decode_values(
    bodies: &[Value],
    now_epoch: i64,
    project: fn(&FileObject) -> Value,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut views = Vec::new();
    for body in bodies {
        let bytes = serde_json::to_vec(body).map_err(failure)?;
        let file = decode_anthropic_file(&bytes, now_epoch).map_err(failure)?;
        views.push(project(&file));
    }
    Ok(views)
}

pub(crate) fn file_list_pagination(case: &ListCase) -> HelperResult {
    let resolved = settings(&case.environment)?;
    let url =
        files_url(case.params_api_base.as_deref(), &resolved, FilesUrl::Files).map_err(failure)?;
    let files = decode_file_list(&case.body, case.now)?;
    Ok(json!({"url": url, "queries": list_queries(&case.purposes), "files": files}))
}

fn decode_file_list(body: &Value, now_epoch: i64) -> Result<Vec<Value>, Box<HelperError>> {
    let bytes = serde_json::to_vec(body).map_err(failure)?;
    let files = decode_anthropic_file_list(&bytes, now_epoch).map_err(failure)?;
    Ok(files.iter().map(file_view).collect())
}

fn list_queries(purposes: &[Option<String>]) -> Vec<Vec<(String, String)>> {
    purposes
        .iter()
        .map(|purpose| purpose_query(purpose.as_deref()))
        .collect()
}

pub(crate) fn file_delete_shape(case: &DeleteCase) -> HelperResult {
    let failure = match &case.failure {
        Some(fixture) => failure_value(fixture)?,
        None => Value::Null,
    };
    let resolved = settings(&case.environment)?;
    let views = url_views(
        &case.file_ids,
        case.params_api_base.as_deref(),
        &resolved,
        UrlKind::File,
    );
    let responses = deleted_replies(&case.bodies)?;
    Ok(json!({
        "retrieve": views,
        "delete": views,
        "responses": responses,
        "failure": failure,
    }))
}

fn failure_value(fixture: &FailureFixture) -> Result<Value, Box<HelperError>> {
    let reply = UpstreamResponse {
        status: fixture.status,
        headers: header_map(&fixture.headers)?,
        body: fixture.body.clone().into_bytes(),
    };
    Ok(status_view(status_failure(&reply).as_ref()))
}

fn status_view(error: Option<&FilesError>) -> Value {
    let Some(FilesError::Provider(provider)) = error else {
        return Value::Null;
    };
    let headers = provider.headers.clone().unwrap_or_default();
    json!({
        "kind": "status",
        "status": provider.status_code,
        "message": provider.message,
        "headers": pairs_object(&headers),
    })
}

fn url_views(
    file_ids: &[String],
    api_base: Option<&str>,
    settings: &AnthropicSettings,
    kind: UrlKind,
) -> Vec<Value> {
    let mut views = Vec::new();
    for file_id in file_ids {
        let target = match kind {
            UrlKind::File => FilesUrl::File(file_id),
            UrlKind::Content => FilesUrl::Content(file_id),
        };
        views.push(match files_url(api_base, settings, target) {
            Ok(url) => json!({"url": url, "query": []}),
            Err(error) => json!({"raised": raised(&error)}),
        });
    }
    views
}

fn deleted_replies(bodies: &[Value]) -> Result<Vec<Value>, Box<HelperError>> {
    bodies
        .iter()
        .map(|body| {
            let bytes = serde_json::to_vec(body).map_err(failure)?;
            let deleted = decode_anthropic_delete(&bytes).map_err(failure)?;
            Ok(deleted_view(&deleted))
        })
        .collect()
}

pub(crate) fn file_content_bytes(case: &ContentCase) -> HelperResult {
    let resolved = settings(&case.environment)?;
    let urls = url_views(
        &case.file_ids,
        case.params_api_base.as_deref(),
        &resolved,
        UrlKind::Content,
    );
    let result = content_result(case)?;
    let error = error_view(&case.error);
    Ok(json!({"urls": urls, "result": result, "error": error}))
}

fn content_result(case: &ContentCase) -> Result<Value, Box<HelperError>> {
    let reply = reply_of(case)?;
    let content = BinaryContent::from_response(reply);
    Ok(json!({"status": content.status, "headers": content.headers, "body": content.body}))
}

fn reply_of(case: &ContentCase) -> Result<UpstreamResponse, Box<HelperError>> {
    let headers = header_map(&case.headers)?;
    Ok(UpstreamResponse {
        status: case.status,
        headers,
        body: case.body.clone(),
    })
}

fn error_view(error: &ErrorFixture) -> Value {
    let failure = response_error(error.status, error.message.clone(), error.headers.clone());
    let headers = failure.headers.clone().unwrap_or_default();
    json!({
        "status": failure.status_code,
        "message": failure.message,
        "headers": pairs_object(&headers)
    })
}

pub(crate) fn file_beta_header(case: &BetaHeaderCase) -> HelperResult {
    let mut scenarios = Vec::new();
    for scenario in &case.scenarios {
        scenarios.push(scenario_view(scenario)?);
    }
    Ok(json!({"scenarios": scenarios}))
}

fn scenario_view(scenario: &BetaScenario) -> Result<Value, Box<HelperError>> {
    let resolved = settings(&scenario.environment)?;
    let urls = scenario_urls(scenario, &resolved)?;
    let (headers, refused) = validated_headers(scenario, &resolved)?;
    let error = error_view(&scenario.error);
    Ok(json!({
        "urls": urls,
        "headers": headers,
        "raised": refused,
        "error": error,
    }))
}

fn scenario_urls(
    scenario: &BetaScenario,
    settings: &AnthropicSettings,
) -> Result<Vec<String>, Box<HelperError>> {
    let mut urls = Vec::new();
    for base in &scenario.url_bases {
        urls.push(files_url(base.as_deref(), settings, FilesUrl::Files).map_err(failure)?);
    }
    Ok(urls)
}

fn validated_headers(
    scenario: &BetaScenario,
    settings: &AnthropicSettings,
) -> Result<(Value, Value), Box<HelperError>> {
    let mut headers = header_map(&scenario.caller_headers)?;
    let api_key = scenario.api_key.clone().map(SecretString::from);
    let validation = validate_environment(
        &mut headers,
        api_key.as_ref(),
        scenario.api_base.as_deref(),
        scenario.params_api_base.as_deref(),
        settings,
    );
    match validation {
        Ok(()) => Ok((headers_view(&headers), Value::Null)),
        Err(error) => Ok((Value::Null, raised(&error))),
    }
}

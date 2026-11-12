use std::collections::BTreeMap;

use betterlite_openai::api::file_type_records::{TwoStepFileUploadConfig, UploadData};
use betterlite_openai::files::records::{FileContentRequest, FileObject, FileUpload};
use betterlite_provider::api::file_contracts::base::BaseConfig as _;
use betterlite_provider::api::file_contracts::config::{
    BaseFilesConfig as _, CompleteUrlRequest, FileLogging, FileParams,
};
use betterlite_provider::manus_files::ManusFilesConfig;
use betterlite_provider::manus_files::create::upload_plan;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::fixture;
use super::{CreatePlan, CreateReply, HeadersAndUrl, PlanCase, ReplyCase, Requests, Sdk};

fn request_view(request: (String, FileParams)) -> Value {
    let (url, query) = request;
    json!({"url": url, "params": query})
}

pub(super) fn headers_and_url(call: &HeadersAndUrl) -> Value {
    let validated: Vec<Value> = call
        .validation
        .iter()
        .map(|case| {
            let config = fixture::config(&case.environment, &case.sdk, 0);
            let key = case.api_key.as_deref().map(SecretString::from);
            let mut headers = case.headers.clone();
            config
                .validate_environment(&mut headers, key.as_ref())
                .map_or_else(
                    |_| json!({"raised": true}),
                    |()| json!({"headers": headers}),
                )
        })
        .collect();
    let urls: Vec<Value> = call
        .urls
        .iter()
        .map(|case| {
            let config = fixture::config(&case.environment, &case.sdk, 0);
            let optional = FileParams::new();
            let request = CompleteUrlRequest {
                api_base: case.api_base.as_deref(),
                api_key: None,
                model: "",
                optional_params: &optional,
                litellm_params: &optional,
            };
            config
                .get_complete_url(request)
                .map_or_else(|_| json!({"raised": true}), |url| json!({"url": url}))
        })
        .collect();
    json!({"validated": validated, "urls": urls})
}

pub(super) fn create_plan(call: &CreatePlan) -> Value {
    let plans: Vec<Value> = call
        .cases
        .iter()
        .map(|case| plan_value(&call.environment, call.now, case))
        .collect();
    json!({"plans": plans})
}

fn plan_value(environment: &BTreeMap<String, String>, now: i64, case: &PlanCase) -> Value {
    let config = fixture::config(environment, &case.sdk, now);
    let file = FileUpload {
        filename: case.file.filename.clone(),
        content_type: None,
        data: case.file.content.clone(),
    };
    let supplied = case.file.present.then_some(&file);
    let litellm_params = fixture::params(case.api_base.as_deref(), case.api_key.as_deref());
    upload_plan(&config, supplied, &litellm_params).map_or_else(
        |_| json!({"raised": true}),
        |plan| json!({"plan": plan_view(&plan)}),
    )
}

fn plan_view(plan: &TwoStepFileUploadConfig) -> Value {
    let initial = &plan.initial_request;
    let upload = &plan.upload_request;
    json!({
        "initial": {
            "method": initial.method,
            "url": initial.url,
            "headers": initial.headers,
            "data": upload_data_view(&initial.data),
        },
        "upload": {
            "method": upload.method,
            "url": upload.url,
            "headers": upload.headers,
            "data": upload_data_view(&upload.data),
        },
        "location": plan.upload_url_location,
        "key": plan.upload_url_key,
    })
}

fn upload_data_view(data: &UploadData) -> Value {
    match data {
        UploadData::Text(text) => json!(text),
        UploadData::Bytes(bytes) => json!(bytes),
        UploadData::Provider(payload) => json!({"filename": payload.value.filename}),
    }
}

pub(super) fn create_reply(call: &CreateReply) -> Value {
    let sdk = Sdk {
        api_base: None,
        api_key: None,
    };
    let config = fixture::config(&call.environment, &sdk, call.now);
    let files: Vec<Value> = call
        .cases
        .iter()
        .map(|case| reply_value(&config, case))
        .collect();
    json!({"files": files})
}

fn reply_value(config: &ManusFilesConfig, case: &ReplyCase) -> Value {
    let raw = fixture::response(&case.body);
    let mut logging = FileLogging::default();
    config
        .transform_create_file_response(None, &raw, &mut logging, &case.litellm_params)
        .map_or_else(|_| json!({"raised": true}), |file| file_view(&file))
}

fn file_view(file: &FileObject) -> Value {
    json!({
        "id": file.id,
        "bytes": file.bytes,
        "created_at": file.created_at,
        "filename": file.filename,
        "object": file.object,
        "purpose": file.purpose,
        "status": file.status,
        "status_details": file.status_details,
    })
}

pub(super) fn requests(call: &Requests) -> Value {
    let config = fixture::config(&call.environment, &call.sdk, call.now);
    let litellm_params = fixture::params(call.api_base.as_deref(), None);
    let empty = FileParams::new();
    let mut retrieve = Vec::with_capacity(call.file_ids.len());
    let mut delete = Vec::with_capacity(call.file_ids.len());
    let mut content = Vec::with_capacity(call.file_ids.len());
    for file_id in &call.file_ids {
        let (retrieved, removed, downloaded) = file_requests(&config, file_id, &litellm_params);
        retrieve.push(retrieved);
        delete.push(removed);
        content.push(downloaded);
    }
    let list_requests: Vec<Value> = call
        .purposes
        .iter()
        .map(|purpose| {
            config
                .transform_list_files_request(purpose.as_deref(), &empty, &litellm_params)
                .map_or_else(|_| json!({"raised": true}), request_view)
        })
        .collect();
    json!({
        "retrieve": retrieve,
        "delete": delete,
        "deleted": deleted_views(&config, &call.delete_bodies, &empty),
        "list_requests": list_requests,
        "files": files_view(&config, &call.body, &empty),
        "content": content,
    })
}

fn deleted_views(config: &ManusFilesConfig, bodies: &[String], params: &FileParams) -> Vec<Value> {
    bodies
        .iter()
        .map(|body| {
            let raw = fixture::response(body);
            let mut logging = FileLogging::default();
            config
                .transform_delete_file_response(&raw, &mut logging, params)
                .map_or_else(
                    |_| json!({"raised": true}),
                    |record| {
                        json!({
                            "id": record.id,
                            "object": record.object,
                            "deleted": record.deleted,
                        })
                    },
                )
        })
        .collect()
}

fn files_view(config: &ManusFilesConfig, body: &str, params: &FileParams) -> Value {
    let raw = fixture::response(body);
    let mut logging = FileLogging::default();
    config
        .transform_list_files_response(&raw, &mut logging, params)
        .map_or_else(
            |_| json!({"raised": true}),
            |listed| json!(listed.iter().map(file_view).collect::<Vec<Value>>()),
        )
}

fn file_requests(
    config: &ManusFilesConfig,
    file_id: &str,
    litellm_params: &FileParams,
) -> (Value, Value, Value) {
    let optional = FileParams::new();
    let retrieve = config
        .transform_retrieve_file_request(file_id, &optional, litellm_params)
        .map_or_else(|_| json!({"raised": true}), request_view);
    let delete = config
        .transform_delete_file_request(file_id, &optional, litellm_params)
        .map_or_else(|_| json!({"raised": true}), request_view);
    let content_request = FileContentRequest {
        file_id: file_id.to_owned(),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    let content = config
        .transform_file_content_request(&content_request, &optional, litellm_params)
        .map_or_else(|_| json!({"raised": true}), request_view);
    (retrieve, delete, content)
}

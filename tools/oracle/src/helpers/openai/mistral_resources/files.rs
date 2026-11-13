use betterlite_config::sections::mistral_resources::Settings;
use betterlite_openai::files::records::FileObject;
use betterlite_openai::mistral_files::transformation::{
    FileUploadRequest, create_upload, file_url, list_request, to_file_deleted, to_file_object,
};
use betterlite_openai::mistral_identity::auth_headers;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use http::header::AUTHORIZATION;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{FilesCall, UploadCall, UploadFile};

fn file_data(upload: &UploadFile) -> FileData {
    let content = FileBody::Bytes(upload.content.clone());
    if let Some(content_type) = &upload.content_type {
        return FileData::Tuple3 {
            filename: upload.filename.clone(),
            content,
            content_type: Some(content_type.clone()),
        };
    }
    if let Some(filename) = &upload.filename {
        return FileData::Tuple2 {
            filename: Some(filename.clone()),
            content,
        };
    }
    FileData::Direct(content)
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
        "expires_at": file.expires_at,
        "status_details": file.status_details,
    })
}

fn upload_item(call: &FilesCall, upload: &UploadCall, settings: &Settings) -> Value {
    let request = FileUploadRequest {
        file: file_data(&upload.file),
        purpose: Some(upload.purpose.clone()),
    };
    match create_upload(&request, call.now, call.api_base.as_deref(), settings) {
        Ok(created) => json!({
            "ok": true,
            "url": created.url,
            "filename": created.filename,
            "content_type": created.content_type,
            "content": created.content,
            "purpose": created.purpose.wire_name(),
        }),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn list_item(purpose: Option<&str>, call: &FilesCall, settings: &Settings) -> Value {
    match list_request(purpose, call.api_base.as_deref(), settings) {
        Ok((url, query)) => json!({"ok": true, "url": url, "query": query}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn delete_item(file_id: &str, call: &FilesCall, settings: &Settings) -> Value {
    match file_url(file_id, "", call.api_base.as_deref(), settings) {
        Ok(url) => json!({"ok": true, "url": url, "query": []}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

pub(super) fn outcome(call: &FilesCall, settings: &Settings) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    let Ok(headers) = auth_headers(&Headers::new(), key.as_ref(), settings) else {
        return json!({"ok": false, "status": null});
    };
    let mut uploads = Vec::with_capacity(call.uploads.len());
    for upload in &call.uploads {
        uploads.push(upload_item(call, upload, settings));
    }
    let mut listed = Vec::with_capacity(call.list_purposes.len());
    for purpose in &call.list_purposes {
        listed.push(list_item(purpose.as_deref(), call, settings));
    }
    let mut bodies = Vec::with_capacity(call.bodies.len());
    for body in &call.bodies {
        bodies.push(file_view(&to_file_object(body, call.now)));
    }
    let mut deleted = Vec::with_capacity(call.deleted.len());
    for body in &call.deleted {
        deleted.push(json!(to_file_deleted(body)));
    }
    let mut removes = Vec::with_capacity(call.file_ids.len());
    for file_id in &call.file_ids {
        removes.push(delete_item(file_id, call, settings));
    }
    json!({
        "ok": true,
        "authorization": headers.get(AUTHORIZATION.as_str()),
        "uploads": uploads,
        "list": listed,
        "bodies": bodies,
        "deleted": deleted,
        "delete": removes,
    })
}

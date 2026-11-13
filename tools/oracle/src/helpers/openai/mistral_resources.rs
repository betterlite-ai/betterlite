use betterlite_config::sections::mistral_resources::Settings;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::mistral_batches::records::MistralBatchJob;
use betterlite_openai::mistral_batches::transformation::{
    create_batch, retrieve_batch, to_batch_object,
};
use betterlite_openai::mistral_files::records::MistralFile;
use betterlite_openai::mistral_files::transformation::{
    FileUploadRequest, create_upload, list_request, to_file_object,
};
use betterlite_openai::mistral_identity::auth_headers;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use http::header::AUTHORIZATION;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
mod tests;
#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    env: BTreeMap<String, String>,
    files: Option<FilesSection>,
    #[serde(default)]
    creates: Vec<CreateSection>,
    #[serde(default)]
    jobs: Vec<MistralBatchJob>,
    #[serde(default)]
    retrieve: Vec<RetrieveCall>,
}
#[derive(Debug, Deserialize)]
struct FilesSection {
    api_base: Option<String>,
    now: i64,
    #[serde(default)]
    uploads: Vec<Upload>,
    #[serde(default)]
    list_purposes: Vec<Option<String>>,
    #[serde(default)]
    bodies: Vec<MistralFile>,
}
#[derive(Debug, Deserialize)]
struct Upload {
    filename: Option<String>,
    content_type: Option<String>,
    content: Vec<u8>,
    purpose: String,
}
#[derive(Debug, Deserialize)]
struct CreateSection {
    api_base: Option<String>,
    api_key: Option<String>,
    model: String,
    data: CreateBatchRequest,
}
#[derive(Debug, Deserialize)]
struct RetrieveCall {
    batch_id: String,
    api_base: Option<String>,
    api_key: Option<String>,
}
fn resource_settings(env: &BTreeMap<String, String>) -> Settings {
    Settings {
        api_base: env
            .get("MISTRAL_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned(),
        api_key: env
            .get("MISTRAL_API_KEY")
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.as_str())),
    }
}
fn upload_outcome(upload: &Upload, section: &FilesSection, settings: &Settings) -> Value {
    let request = FileUploadRequest {
        file: FileData::Tuple3 {
            filename: upload.filename.clone(),
            content: FileBody::Bytes(upload.content.clone()),
            content_type: upload.content_type.clone(),
        },
        purpose: Some(upload.purpose.clone()),
    };
    match create_upload(&request, section.now, section.api_base.as_deref(), settings) {
        Ok(created) => {
            json!({"ok": true, "url": created.url, "filename": created.filename, "content_type": created.content_type, "content": created.content, "purpose": created.purpose.wire_name()})
        }
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}
fn list_outcome(purpose: Option<&str>, section: &FilesSection, settings: &Settings) -> Value {
    match list_request(purpose, section.api_base.as_deref(), settings) {
        Ok((url, query)) => json!({"ok": true, "url": url, "query": query}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}
fn files_outcome(section: &FilesSection, settings: &Settings) -> Value {
    let mut uploads = Vec::with_capacity(section.uploads.len());
    for upload in &section.uploads {
        uploads.push(upload_outcome(upload, section, settings));
    }
    let mut listed = Vec::with_capacity(section.list_purposes.len());
    for purpose in &section.list_purposes {
        listed.push(list_outcome(purpose.as_deref(), section, settings));
    }
    let mut bodies = Vec::with_capacity(section.bodies.len());
    for body in &section.bodies {
        bodies.push(json!(to_file_object(body, section.now)));
    }
    json!({"uploads": uploads, "list": listed, "bodies": bodies})
}
fn create_outcome(section: &CreateSection, settings: &Settings) -> Value {
    let key = section.api_key.as_deref().map(SecretString::from);
    let headers = auth_headers(&Headers::new(), key.as_ref(), settings);
    let api_base = section.api_base.as_deref();
    let plan = create_batch(&section.data, &section.model, api_base, settings);
    if let (Ok(headers), Ok(plan)) = (headers, plan) {
        return json!({"ok": true, "url": plan.url, "authorization": headers.get(AUTHORIZATION.as_str()), "body": plan.body});
    }
    json!({"ok": false, "status": null})
}
fn retrieve_outcome(call: &RetrieveCall, settings: &Settings) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    match retrieve_batch(
        &call.batch_id,
        call.api_base.as_deref(),
        key.as_ref(),
        settings,
    ) {
        Ok(request) => {
            json!({"ok": true, "method": request.method.as_str(), "url": request.url, "headers": request.headers})
        }
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let settings = resource_settings(&input.env);
        let files = input
            .files
            .as_ref()
            .map(|section| files_outcome(section, &settings));
        let mut create = Vec::with_capacity(input.creates.len());
        for section in &input.creates {
            create.push(create_outcome(section, &settings));
        }
        let mut jobs = Vec::with_capacity(input.jobs.len());
        for job in &input.jobs {
            jobs.push(json!({"ok": true, "batch": to_batch_object(job)}));
        }
        let mut retrieve = Vec::with_capacity(input.retrieve.len());
        for call in &input.retrieve {
            retrieve.push(retrieve_outcome(call, &settings));
        }
        Ok(json!({"files": files, "create": create, "jobs": jobs, "retrieve": retrieve}))
    }
}

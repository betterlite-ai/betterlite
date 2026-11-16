use std::collections::BTreeMap;
use std::future::ready;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_openai::files::records::{CreatePurpose, FileObject};
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::vertex_files::FileRequest;
use betterlite_vertex::vertex_files::auth::{EnvironmentRequest, validate_environment};
use betterlite_vertex::vertex_files::error::VertexFilesError;
use betterlite_vertex::vertex_files::naming::get_object_name;
use betterlite_vertex::vertex_files::records::{
    MetadataShape, decode_vertex_file_metadata, deleted_file,
};
use betterlite_vertex::vertex_files::targets::upload_url;
use betterlite_vertex::vertex_gcs_access::bucket::BucketCandidates;
use betterlite_vertex::vertex_interactions::mint::{MintAccessToken, MintFuture};
use jiff::tz::TimeZone;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ObjectName(NameCall),
    UploadUrl(NameCall),
    Headers {
        headers: BTreeMap<String, String>,
        api_key: String,
    },
    UploadReply {
        body: String,
    },
    RetrieveReply {
        body: String,
    },
    DeleteReply {
        url: Option<String>,
    },
    List {},
}

// the naming inputs both preparation operations read; upload_url also reads the bucket and the host
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NameCall {
    file: FileInput,
    purpose: CreatePurpose,
    deployment_model: Option<String>,
    #[serde(default)]
    passthrough: bool,
    #[serde(default)]
    custom_endpoint: bool,
    bucket: Option<String>,
    api_base: Option<String>,
    uuid: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileInput {
    content: String,
    filename: Option<String>,
    content_type: Option<String>,
}

struct NoMint;

impl MintAccessToken for NoMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        _project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        Box::pin(ready(Err(CredentialCacheError::MissingCredentials)))
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::ObjectName(call) => prepared(call, Answer::Name),
        Input::UploadUrl(call) => prepared(call, Answer::Url),
        Input::Headers { headers, api_key } => environment_headers(headers, api_key),
        Input::UploadReply { body } => file_reply(body, MetadataShape::Upload),
        Input::RetrieveReply { body } => file_reply(body, MetadataShape::Retrieve),
        Input::DeleteReply { url } => Ok(delete_record(url.as_deref())),
        Input::List {} => Err(files_error(VertexFilesError::Listing)),
    }
}

fn request<'file>(call: &'file NameCall, data: &'file FileData) -> FileRequest<'file> {
    FileRequest {
        file: data,
        purpose: call.purpose,
        deployment_model: call.deployment_model.as_deref(),
        passthrough: call.passthrough,
        custom_endpoint: call.custom_endpoint,
    }
}

// the two preparation calls read the same body and the same fixed id, and answer with one string
#[derive(Clone, Copy)]
enum Answer {
    Name,
    Url,
}

fn prepared(call: &NameCall, answer: Answer) -> HelperResult {
    let data = file_data(&call.file);
    let file_request = request(call, &data);
    let id = Uuid::from_u128(u128::from(call.uuid));
    match answer {
        Answer::Name => {
            let name = get_object_name(&file_request, id).map_err(files_error)?;
            Ok(json!({ "name": name }))
        }
        Answer::Url => {
            let candidates = BucketCandidates {
                deployment: call.bucket.clone(),
                deployment_alias: None,
                batch_environment: None,
                environment: None,
            };
            let url = upload_url(&file_request, call.api_base.as_deref(), &candidates, id)
                .map_err(files_error)?;
            Ok(json!({ "url": url }))
        }
    }
}

fn environment_headers(headers: &BTreeMap<String, String>, api_key: &str) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let key = SecretString::from(api_key.to_owned());
    let environment = EnvironmentRequest {
        headers: headers.clone(),
        api_key: Some(&key),
        credentials: None,
        project: None,
    };
    let validated = runtime
        .block_on(validate_environment(environment, &NoMint))
        .map_err(files_error)?;
    let exposed = validated
        .into_iter()
        .map(|(name, value)| (name, value.expose_secret().to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(json!({ "headers": exposed }))
}

fn file_reply(body: &str, shape: MetadataShape) -> HelperResult {
    let local = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let file = decode_vertex_file_metadata(body.as_bytes(), shape, &local).map_err(files_error)?;
    Ok(json!({ "file": file_record(&file) }))
}

fn file_record(file: &FileObject) -> Value {
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

fn delete_record(url: Option<&str>) -> Value {
    let deleted = deleted_file(url);
    json!({
        "id": deleted.id,
        "object": deleted.object,
        "deleted": deleted.deleted,
    })
}

fn file_data(file: &FileInput) -> FileData {
    let content = FileBody::Bytes(file.content.clone().into_bytes());
    match (&file.filename, &file.content_type) {
        (filename, Some(content_type)) => FileData::Tuple3 {
            filename: filename.clone(),
            content,
            content_type: Some(content_type.clone()),
        },
        (Some(filename), None) => FileData::Tuple2 {
            filename: Some(filename.clone()),
            content,
        },
        (None, None) => FileData::Direct(content),
    }
}

fn files_error(error: VertexFilesError) -> Box<HelperError> {
    let class = if matches!(error, VertexFilesError::Listing) {
        "NotImplementedError"
    } else {
        "VertexAIError"
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

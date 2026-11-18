use betterlite_azure::azure_files::client::SelectedClient;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::endpoints::projection::ClientKind;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{BinaryContent, FileDeleted, FileObject, FilePage};
use http::HeaderMap;
use serde_json::{Map, Value, json};

// The headers both sides compare: what the file wrappers own, plus the case's own x- headers
const COMPARED: [&str; 4] = ["accept", "api-key", "authorization", "openai-organization"];

pub(super) fn request(request: &OutboundRequest) -> Value {
    json!({
        "method": request.method.as_str(),
        "host": request.url.host_str(),
        "path": request.url.path(),
        "query": request
            .query
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect::<Vec<_>>(),
        "headers": headers(&request.headers),
        "timeout_ms": request.deadline.as_millis(),
        "parts": parts(&request.body),
    })
}

fn headers(headers: &HeaderMap) -> Value {
    let mut selected = Map::new();
    for (name, value) in headers {
        let name = name.as_str();
        if COMPARED.contains(&name) || (name.starts_with("x-") && !name.starts_with("x-stainless-"))
        {
            selected.insert(name.to_owned(), json!(value.to_str().unwrap_or_default()));
        }
    }
    Value::Object(selected)
}

// The multipart fields in the order the request builder writes them; null for every other method.
// A part without a filename or content type is an empty string, as the python adapter's regex reads it.
fn parts(body: &RequestBody) -> Value {
    let RequestBody::Multipart { parts, .. } = body else {
        return Value::Null;
    };
    Value::Array(
        parts
            .iter()
            .map(|part| {
                json!({
                    "name": &part.name,
                    "filename": part.file_name.as_deref().unwrap_or_default(),
                    "content_type": part.content_type.as_deref().unwrap_or_default(),
                    "data": std::str::from_utf8(&part.data).unwrap_or_default(),
                })
            })
            .collect(),
    )
}

pub(super) fn object(object: &FileObject) -> Value {
    json!({
        "id": &object.id, "object": object.object, "bytes": object.bytes,
        "created_at": object.created_at, "filename": &object.filename, "purpose": object.purpose,
        "status": &object.status, "expires_at": object.expires_at,
        "status_details": &object.status_details,
    })
}

pub(super) fn page(page: &FilePage) -> Value {
    json!({
        "object": page.object, "data": page.data.iter().map(object).collect::<Vec<_>>(),
        "has_more": page.has_more, "first_id": &page.first_id, "last_id": &page.last_id,
    })
}

pub(super) fn deleted(deleted: &FileDeleted) -> Value {
    json!({ "id": &deleted.id, "object": deleted.object, "deleted": deleted.deleted })
}

pub(super) fn content(content: &BinaryContent) -> Value {
    json!({
        "status": content.status, "headers": &content.headers,
        "body": std::str::from_utf8(&content.body).unwrap_or_default(),
    })
}

pub(super) fn client(client: &SelectedClient) -> Value {
    json!({
        "kind": kind_name(client.kind), "base_url": client.files.base_url.as_str(),
        "query": query(&client.query()), "timeout_ms": client.files.timeout.as_millis(),
        "max_retries": client.files.max_retries,
    })
}

fn query(query: &[(String, String)]) -> Value {
    let mut fields = Map::new();
    for (name, value) in query {
        fields.insert(name.clone(), json!(value));
    }
    Value::Object(fields)
}

const fn kind_name(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::AzureOpenAi => "AzureOpenAI",
        ClientKind::AsyncAzureOpenAi => "AsyncAzureOpenAI",
        ClientKind::OpenAi => "OpenAI",
        ClientKind::AsyncOpenAi => "AsyncOpenAI",
        ClientKind::Other => "Other",
    }
}

pub(super) fn raised(error: &AzureFilesError) -> Value {
    let (class, status, error_type, param, code, message) = match error {
        AzureFilesError::Files {
            source: FilesError::Status(status),
        } => (
            "APIStatusError",
            json!(status.status),
            json!(&status.error_type),
            json!(&status.param),
            json!(&status.code),
            Value::Null,
        ),
        AzureFilesError::Files {
            source: FilesError::Decode { .. },
        } => (
            "ValidationError",
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
        ),
        AzureFilesError::MissingCredentials => (
            "OpenAIError",
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            json!(error.to_string()),
        ),
        _ => (
            "ValueError",
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            json!(error.to_string()),
        ),
    };
    json!({
        "class": class, "status": status, "type": error_type,
        "param": param, "code": code, "message": message,
    })
}

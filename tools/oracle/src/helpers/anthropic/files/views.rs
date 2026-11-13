use std::collections::BTreeMap;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_openai::files::records::{FileDeleted, FileObject};
use http::HeaderMap;
use serde_json::{Value, json};

pub(crate) fn file_view(file: &FileObject) -> Value {
    json!({
        "id": file.id, "bytes": file.bytes, "created_at": file.created_at, "filename": file.filename, "object": file.object, "purpose": file.purpose, "status": file.status, "status_details": file.status_details, "expires_at": file.expires_at,
    })
}

pub(crate) fn deleted_view(deleted: &FileDeleted) -> Value {
    json!({
        "id": deleted.id, "object": deleted.object, "deleted": deleted.deleted,
    })
}

pub(crate) fn parts_view(parts: &[MultipartPart]) -> Value {
    parts
        .iter()
        .map(|part| json!({"name": part.name, "filename": part.file_name, "content_type": part.content_type, "content": part.data}))
        .collect()
}

pub(crate) fn headers_view(headers: &HeaderMap) -> Value {
    json!(
        headers
            .iter()
            .map(|(name, value)| (
                name.as_str().to_owned(),
                value
                    .to_str()
                    .map_or_else(|_invalid| String::new(), str::to_owned)
            ))
            .collect::<BTreeMap<String, String>>()
    )
}

pub(crate) fn pairs_object(pairs: &[(String, String)]) -> Value {
    let mut object = BTreeMap::new();
    for (name, value) in pairs {
        object.insert(name.clone(), value.clone());
    }
    json!(object)
}

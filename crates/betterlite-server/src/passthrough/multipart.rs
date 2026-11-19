use betterlite_core::api::request_form::values::{FormParts, PartValue, UploadFile};
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::{CONTENT_TYPE, HeaderMap};

use super::upstream::UpstreamTarget;

const MULTIPART_MARKER: &[u8] = b"multipart/form-data";

// LiteLLM's is_multipart: the media type only has to appear inside the header value
#[must_use]
pub fn is_multipart(headers: &HeaderMap) -> bool {
    headers.get(CONTENT_TYPE).is_some_and(|value| {
        value
            .as_bytes()
            .windows(MULTIPART_MARKER.len())
            .any(|window| window == MULTIPART_MARKER)
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct MultipartFile {
    pub field_name: String,
    pub filename: Option<String>,
    pub content: Vec<u8>,
    pub content_type: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MultipartField {
    pub field_name: String,
    pub values: Vec<String>,
}

// The files= and data= arguments httpx receives: files carry what the upload declared, data groups repeated fields
#[derive(Debug, PartialEq, Eq)]
pub struct MultipartForm {
    pub files: Vec<MultipartFile>,
    pub data: Option<Vec<MultipartField>>,
}

impl MultipartForm {
    // multi_items() order survives, so repeated names reach httpx in the order the client sent them
    #[must_use]
    pub fn from_parts(form: &FormParts) -> Self {
        let mut files = Vec::new();
        let mut non_file_items: Vec<(String, String)> = Vec::new();
        for part in &form.items {
            match &part.value {
                PartValue::Upload(upload) => {
                    files.push(request_file_from_upload(&part.name, upload));
                }
                PartValue::Text(value) => non_file_items.push((part.name.clone(), value.clone())),
            }
        }
        if files.is_empty() {
            let fileless = non_file_items
                .into_iter()
                .map(|(field_name, value)| MultipartFile {
                    field_name,
                    filename: None,
                    content: value.into_bytes(),
                    content_type: None,
                })
                .collect();
            return Self {
                files: fileless,
                data: None,
            };
        }
        Self {
            files,
            data: Some(grouped_fields(&non_file_items)),
        }
    }

    // httpx writes the data fields first and the file entries after them
    fn into_parts(self) -> Vec<MultipartPart> {
        let mut parts = Vec::new();
        for field in self.data.unwrap_or_default() {
            for value in field.values {
                parts.push(MultipartPart {
                    name: field.field_name.clone(),
                    file_name: None,
                    content_type: None,
                    data: value.into_bytes(),
                });
            }
        }
        for file in self.files {
            let content_type = rendered_content_type(&file);
            parts.push(MultipartPart {
                name: file.field_name,
                file_name: file.filename.filter(|file_name| !file_name.is_empty()),
                content_type,
                data: file.content,
            });
        }
        parts
    }
}

// httpx guesses a missing part type from the file name, and sends none for an unnamed part
fn rendered_content_type(file: &MultipartFile) -> Option<String> {
    if file.content_type.is_some() {
        return file.content_type.clone();
    }
    let file_name = file.filename.as_deref()?;
    if file_name.is_empty() {
        return None;
    }
    Some(
        mime_guess::from_path(file_name)
            .first_raw()
            .unwrap_or("application/octet-stream")
            .to_owned(),
    )
}

// a dict of lists in first-appearance order, the shape httpx turns into repeated DataFields
fn grouped_fields(items: &[(String, String)]) -> Vec<MultipartField> {
    let mut fields: Vec<MultipartField> = Vec::new();
    for (name, value) in items {
        match fields
            .iter_mut()
            .find(|field| field.field_name.as_str() == name.as_str())
        {
            Some(field) => field.values.push(value.clone()),
            None => fields.push(MultipartField {
                field_name: name.clone(),
                values: vec![value.clone()],
            }),
        }
    }
    fields
}

// LiteLLM's _build_request_files_from_upload_file: the upload already holds what read() returned
#[must_use]
pub fn request_file_from_upload(field_name: &str, upload: &UploadFile) -> MultipartFile {
    MultipartFile {
        field_name: field_name.to_owned(),
        filename: Some(upload.filename.clone()),
        content: upload.content.clone(),
        content_type: upload.content_type.clone(),
    }
}

// LiteLLM's make_multipart_http_request: the caller's content-type goes so the rebuilt boundary wins
#[must_use]
pub fn make_multipart_http_request(
    target: UpstreamTarget,
    form: &FormParts,
    boundary: &str,
) -> OutboundRequest {
    let UpstreamTarget {
        method,
        url,
        query,
        mut headers,
        deadline,
    } = target;
    headers.remove(CONTENT_TYPE);
    let parts = MultipartForm::from_parts(form).into_parts();
    let body = if parts.is_empty() {
        RequestBody::Empty
    } else {
        RequestBody::Multipart {
            boundary: boundary.to_owned(),
            parts,
        }
    };
    OutboundRequest {
        method,
        url,
        query,
        headers,
        body,
        deadline,
    }
}

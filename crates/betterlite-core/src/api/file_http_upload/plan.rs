use std::collections::BTreeMap;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_logging::logging_pre_post::masking::masked_headers;
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::{
    ByteStream, ProviderUploadPayload, UploadData, UploadUrlLocation,
};
use indexmap::IndexMap;
use serde_json::Number;

// What transform_create_file_request hands the upload service: one shape per LiteLLM branch
pub enum UploadPlan {
    TwoStep(Box<TwoStepPlan>),
    Presigned(Box<PresignedPlan>),
    Media(Box<MediaPlan>),
    Text(String),
    Bytes(Vec<u8>),
    Multipart(Box<MultipartPlan>),
    Unsupported(Box<Json>),
}

// The two-step upload: an initial request asks for the URL, the second one puts the file there
pub struct TwoStepPlan {
    pub initial_method: Option<String>,
    pub initial_url: Option<String>,
    pub initial_headers: BTreeMap<String, String>,
    pub initial_data: Json,
    pub upload_method: Option<String>,
    pub upload_url: Option<String>,
    pub upload_headers: BTreeMap<String, String>,
    pub upload_data: UploadData,
    pub url_location: Option<UploadUrlLocation>,
    pub url_key: Option<String>,
}

// Bedrock's S3 form: every request field comes from the provider transform, untouched
pub struct PresignedPlan {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub data: UploadData,
}

// Vertex's single streaming upload: the body is produced block by block off the executor
pub struct MediaPlan {
    pub body_stream: ByteStream,
    pub content_type: Option<String>,
}

// Anthropic's form upload, already split into the parts httpx's files= mapping would encode
pub struct MultipartPlan {
    pub parts: Vec<MultipartPart>,
}

impl UploadPlan {
    // The value pre_call logs as complete_input_dict, with presigned headers masked and media a placeholder
    #[must_use]
    pub fn logged_input(&self) -> Option<Json> {
        match self {
            Self::TwoStep(plan) => {
                let mut logged = IndexMap::new();
                logged.insert(
                    String::from("initial_request"),
                    request_json(
                        plan.initial_method.as_deref(),
                        plan.initial_url.as_deref(),
                        &plan.initial_headers,
                        plan.initial_data.clone(),
                    ),
                );
                logged.insert(
                    String::from("upload_request"),
                    request_json(
                        plan.upload_method.as_deref(),
                        plan.upload_url.as_deref(),
                        &plan.upload_headers,
                        upload_data_json(&plan.upload_data),
                    ),
                );
                if let Some(location) = plan.url_location {
                    logged.insert(String::from("upload_url_location"), location_json(location));
                }
                if let Some(key) = &plan.url_key {
                    logged.insert(String::from("upload_url_key"), Json::Text(key.clone()));
                }
                Some(Json::Object(logged))
            }
            Self::Presigned(plan) => Some(presigned_logged(plan)),
            Self::Media(_) => Some(Json::Text(String::from("<streaming media upload>"))),
            Self::Text(text) => Some(Json::Text(text.clone())),
            Self::Multipart(plan) => Some(multipart_logged(plan)),
            Self::Unsupported(plan) => Some((**plan).clone()),
            // a Python bytes body has no JSON form; the log carries none rather than a made-up one
            Self::Bytes(_) => None,
        }
    }
}

fn presigned_logged(plan: &PresignedPlan) -> Json {
    let mut logged = IndexMap::new();
    logged.insert(String::from("method"), Json::Text(plan.method.clone()));
    logged.insert(String::from("url"), Json::Text(plan.url.clone()));
    let headers = masked_headers(&headers_json(&plan.headers), false);
    logged.insert(String::from("headers"), Json::Object(headers));
    logged.insert(String::from("data"), upload_data_json(&plan.data));
    Json::Object(logged)
}

fn multipart_logged(plan: &MultipartPlan) -> Json {
    let mut form = IndexMap::new();
    for part in &plan.parts {
        let name = part
            .file_name
            .as_deref()
            .map_or(Json::Null, |text| Json::Text(text.to_owned()));
        let mut tuple = vec![name, bytes_json(&part.data)];
        if let Some(content_type) = &part.content_type {
            tuple.push(Json::Text(content_type.clone()));
        }
        form.insert(part.name.clone(), Json::List(tuple));
    }
    Json::Object(form)
}

fn request_json(
    method: Option<&str>,
    url: Option<&str>,
    headers: &BTreeMap<String, String>,
    data: Json,
) -> Json {
    let mut request = IndexMap::new();
    if let Some(method) = method {
        request.insert(String::from("method"), Json::Text(method.to_owned()));
    }
    if let Some(url) = url {
        request.insert(String::from("url"), Json::Text(url.to_owned()));
    }
    request.insert(String::from("headers"), Json::Object(headers_json(headers)));
    request.insert(String::from("data"), data);
    Json::Object(request)
}

fn headers_json(headers: &BTreeMap<String, String>) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// The wire bytes of an upload body: encoded text, raw bytes, or the object a provider payload means
pub fn encoded_body(data: &UploadData) -> Result<Vec<u8>, serde_json::Error> {
    match data {
        UploadData::Text(text) => Ok(text.clone().into_bytes()),
        UploadData::Bytes(bytes) => Ok(bytes.clone()),
        UploadData::Provider(payload) => serde_json::to_vec(&provider_json(payload)),
    }
}

// The typed provider record holding the initial body: JSON text parses back to the object it names
#[must_use]
pub fn initial_data_json(data: &UploadData) -> Json {
    match data {
        UploadData::Text(text) => {
            serde_json::from_str(text).unwrap_or_else(|_error| Json::Text(text.clone()))
        }
        UploadData::Bytes(bytes) => bytes_json(bytes),
        UploadData::Provider(payload) => provider_json(payload),
    }
}

#[must_use]
pub fn upload_data_json(data: &UploadData) -> Json {
    match data {
        UploadData::Text(text) => Json::Text(text.clone()),
        UploadData::Bytes(bytes) => bytes_json(bytes),
        UploadData::Provider(payload) => provider_json(payload),
    }
}

// The object a typed provider payload stands for, which is what both the wire and the log carry
#[must_use]
pub fn provider_json(payload: &ProviderUploadPayload) -> Json {
    let name = payload.value.filename.clone();
    Json::Object(IndexMap::from([(
        String::from("filename"),
        Json::Text(name),
    )]))
}

#[must_use]
pub fn bytes_json(bytes: &[u8]) -> Json {
    Json::List(
        bytes
            .iter()
            .map(|byte| Json::Number(Number::from(*byte)))
            .collect(),
    )
}

fn location_json(location: UploadUrlLocation) -> Json {
    match location {
        UploadUrlLocation::Headers => Json::Text(String::from("headers")),
        UploadUrlLocation::Body => Json::Text(String::from("body")),
    }
}

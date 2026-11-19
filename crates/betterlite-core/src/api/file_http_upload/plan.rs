use std::collections::BTreeMap;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_logging::logging_pre_post::masking::masked_headers;
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::{
    ByteStream, ProviderUploadPayload, UploadData, UploadUrlLocation,
};
use indexmap::IndexMap;
use serde_json::Number;

// What transform_create_file_request hands the upload service: one shape per LiteLLM branch, so an
// object that names no branch is the only open case left
pub enum UploadPlan {
    TwoStep(Box<TwoStepPlan>),
    Presigned(Box<PresignedPlan>),
    Media(Box<MediaPlan>),
    Text(String),
    Bytes(Vec<u8>),
    Multipart(Box<MultipartPlan>),
    Unsupported(Box<Json>),
}

// The two-step upload Gemini, Manus and the GCS resumable path build: the first request asks for a
// URL, the second one puts the file there. LiteLLM always POSTs the initial request to api_base and
// uploads to the URL it read back, so the two request urls are logged but never sent.
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
    // The value pre_call logs as complete_input_dict, with the presigned headers masked and the
    // media payload reduced to a placeholder, exactly as async_create_file builds it
    #[must_use]
    pub fn logged_input(&self) -> Option<Json> {
        match self {
            Self::TwoStep(plan) => Some(plan.logged()),
            Self::Presigned(plan) => Some(plan.logged()),
            Self::Media(_) => Some(Json::Text(String::from("<streaming media upload>"))),
            Self::Text(text) => Some(Json::Text(text.clone())),
            Self::Multipart(plan) => Some(plan.logged()),
            Self::Unsupported(plan) => Some((**plan).clone()),
            // a Python bytes body has no JSON form; the log carries none rather than a made-up one
            Self::Bytes(_) => None,
        }
    }
}

impl TwoStepPlan {
    #[must_use]
    pub fn logged(&self) -> Json {
        let mut plan = IndexMap::new();
        plan.insert("initial_request".to_owned(), self.initial_request());
        plan.insert("upload_request".to_owned(), self.upload_request());
        if let Some(location) = self.url_location {
            plan.insert("upload_url_location".to_owned(), location_json(location));
        }
        if let Some(key) = &self.url_key {
            plan.insert("upload_url_key".to_owned(), Json::Text(key.clone()));
        }
        Json::Object(plan)
    }

    fn initial_request(&self) -> Json {
        let mut request =
            request_identity(self.initial_method.as_deref(), self.initial_url.as_deref());
        request.insert(
            "headers".to_owned(),
            Json::Object(
                self.initial_headers
                    .iter()
                    .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                    .collect(),
            ),
        );
        request.insert("data".to_owned(), self.initial_data.clone());
        Json::Object(request)
    }

    fn upload_request(&self) -> Json {
        let mut request =
            request_identity(self.upload_method.as_deref(), self.upload_url.as_deref());
        request.insert(
            "headers".to_owned(),
            Json::Object(
                self.upload_headers
                    .iter()
                    .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                    .collect(),
            ),
        );
        request.insert("data".to_owned(), upload_data_json(&self.upload_data));
        Json::Object(request)
    }
}

impl PresignedPlan {
    #[must_use]
    pub fn logged(&self) -> Json {
        let mut plan = IndexMap::new();
        plan.insert("method".to_owned(), Json::Text(self.method.clone()));
        plan.insert("url".to_owned(), Json::Text(self.url.clone()));
        plan.insert(
            "headers".to_owned(),
            Json::Object(masked_plan_headers(&self.headers)),
        );
        plan.insert("data".to_owned(), upload_data_json(&self.data));
        Json::Object(plan)
    }
}

impl MultipartPlan {
    #[must_use]
    pub fn logged(&self) -> Json {
        let mut form = IndexMap::new();
        for part in &self.parts {
            let mut tuple = vec![file_name_json(part.file_name.as_deref())];
            tuple.push(bytes_json(&part.data));
            if let Some(content_type) = &part.content_type {
                tuple.push(Json::Text(content_type.clone()));
            }
            form.insert(part.name.clone(), Json::List(tuple));
        }
        Json::Object(form)
    }
}

// The wire bytes of an upload body: a provider's already-encoded text, raw bytes, or the object a
// typed provider payload stands for
pub fn encoded_body(data: &UploadData) -> Result<Vec<u8>, serde_json::Error> {
    match data {
        UploadData::Text(text) => Ok(text.clone().into_bytes()),
        UploadData::Bytes(bytes) => Ok(bytes.clone()),
        UploadData::Provider(payload) => serde_json::to_vec(&provider_json(payload)),
    }
}

// httpx sends a two-step initial body as json.dumps(data), so the plan keeps it as JSON
pub fn initial_body(data: &Json) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(data)
}

// The typed provider record holding the initial body: its text form parses back to the object it
// serialized, and anything that is not JSON text keeps the text it was given
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
    Json::Object(IndexMap::from([(
        "filename".to_owned(),
        Json::Text(payload.value.filename.clone()),
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

fn request_identity(method: Option<&str>, url: Option<&str>) -> IndexMap<String, Json> {
    let mut request = IndexMap::new();
    if let Some(method) = method {
        request.insert("method".to_owned(), Json::Text(method.to_owned()));
    }
    if let Some(url) = url {
        request.insert("url".to_owned(), Json::Text(url.to_owned()));
    }
    request
}

fn location_json(location: UploadUrlLocation) -> Json {
    match location {
        UploadUrlLocation::Headers => Json::Text(String::from("headers")),
        UploadUrlLocation::Body => Json::Text(String::from("body")),
    }
}

fn file_name_json(name: Option<&str>) -> Json {
    name.map_or(Json::Null, |text| Json::Text(text.to_owned()))
}

// LiteLLM masks a presigned request's own headers before pre_call, since logging sees them as body
fn masked_plan_headers(headers: &BTreeMap<String, String>) -> IndexMap<String, Json> {
    let sent: IndexMap<String, Json> = headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    masked_headers(&sent, false)
}

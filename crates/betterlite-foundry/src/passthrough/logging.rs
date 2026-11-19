use betterlite_azure::passthrough::logging::AzureRelayedLog;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponseObject;
use betterlite_ocr::response::OCRResponse;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, RelayCallType, RelayShape,
};
use serde::Deserialize;

// LoggedRelayResponse | OCRResponse | StandardPassThroughResponseObject, one variant per projection
#[derive(Debug, Clone)]
pub enum FoundryRelayedLog {
    Azure(AzureRelayedLog),
    Ocr(Box<OCRResponse>),
    Relay(Object),
    Standard(PassThroughResponseObject),
}

// CallTypes.aocr is not in B3570's RelayCallType, so the OCR path names its own call type
pub trait FoundryPassthroughLogging: PassthroughLogging {
    fn set_ocr_call_type(&mut self);
}

// the trait path of BasePassthroughConfig cannot reach set_ocr_call_type, so that path logs the OCR body as a plain relay
pub(super) struct TraitFoundryLogging<'logging>(pub(super) &'logging mut dyn PassthroughLogging);

impl PassthroughLogging for TraitFoundryLogging<'_> {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.0.set_call_type(call_type);
    }
}

impl FoundryPassthroughLogging for TraitFoundryLogging<'_> {
    fn set_ocr_call_type(&mut self) {}
}

// RerankResponse.model_validate: every field is optional, but a result needs its index and score
#[derive(Debug, Deserialize)]
struct RerankReply {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    results: Option<Vec<RerankResult>>,
    #[serde(default)]
    meta: Option<Object>,
}

#[derive(Debug, Deserialize)]
struct RerankResult {
    index: i64,
    relevance_score: f64,
    #[serde(default)]
    document: Option<RerankDocument>,
}

#[derive(Debug, Deserialize)]
struct RerankDocument {
    text: String,
}

// ImagesResponse: created is required and data, when sent, is a list of image objects
#[derive(Debug, Deserialize)]
struct ImageReply {
    created: i64,
    #[serde(default)]
    data: Option<Vec<ImageEntry>>,
}

#[derive(Debug, Deserialize)]
struct ImageEntry {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    b64_json: Option<String>,
}

pub const FOUNDRY_RELAY_SHAPES: [RelayShape<'static, Object>; 2] = [
    RelayShape {
        path_suffix: "/rerank",
        call_type: RelayCallType::Arerank,
        parse: rerank_response,
    },
    RelayShape {
        path_suffix: "/providers/blackforestlabs/v1/flux-2-pro",
        call_type: RelayCallType::AimageGeneration,
        parse: image_response,
    },
];

fn rerank_response(body: &Object) -> Option<Object> {
    serde_json::from_value::<RerankReply>(serde_json::json!(body))
        .is_ok()
        .then(|| body.clone())
}

fn image_response(body: &Object) -> Option<Object> {
    serde_json::from_value::<ImageReply>(serde_json::json!(body))
        .is_ok()
        .then(|| body.clone())
}

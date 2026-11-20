use betterlite_azure::passthrough::logging::AzureRelayedLog;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponseObject;
use betterlite_ocr::response::OCRResponse;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, RelayCallType, RelayShape,
};

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

// RerankResponse.model_validate: id, results and meta are optional, but a sent result needs its index and score
fn rerank_response(body: &Object) -> Option<Object> {
    let results_fit = body.get("results").is_none_or(|value| {
        value.is_null()
            || value.as_array().is_some_and(|results| {
                results
                    .iter()
                    .all(|result| result.as_object().is_some_and(rerank_result_fits))
            })
    });
    let meta_fits = body
        .get("meta")
        .is_none_or(|value| value.is_null() || value.is_object());
    (optional_text(body, "id") && results_fit && meta_fits).then(|| body.clone())
}

fn rerank_result_fits(fields: &Object) -> bool {
    let index_fits = fields
        .get("index")
        .is_some_and(|index| index.is_i64() || index.is_u64());
    let score_fits = fields
        .get("relevance_score")
        .is_some_and(|score| score.is_number());
    let document_fits = fields.get("document").is_none_or(|document| {
        document
            .as_object()
            .is_some_and(|document| document.get("text").is_some_and(|text| text.is_string()))
    });
    index_fits && score_fits && document_fits
}

// ImagesResponse.model_validate: created is required, and data, when sent, is a list of image objects
fn image_response(body: &Object) -> Option<Object> {
    let created_is_integer = body
        .get("created")
        .is_some_and(|value| value.is_i64() || value.is_u64());
    let data_fits = body.get("data").is_none_or(|value| {
        value.is_null()
            || value.as_array().is_some_and(|entries| {
                entries.iter().all(|entry| {
                    entry.as_object().is_some_and(|fields| {
                        optional_text(fields, "b64_json")
                            && optional_text(fields, "revised_prompt")
                            && optional_text(fields, "url")
                    })
                })
            })
    });
    (created_is_integer && data_fits).then(|| body.clone())
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

fn optional_text(fields: &Object, name: &str) -> bool {
    fields
        .get(name)
        .is_none_or(|value| value.is_null() || value.is_string())
}

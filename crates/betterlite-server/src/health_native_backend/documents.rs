use betterlite_ocr::ocr_document::OcrDocument;
use betterlite_ocr::ocr_provider::get_health_check_document;

use crate::health_probe_modes::backend::OcrHealthDocuments;
use crate::health_probe_modes::error::HealthProbeError;

// betterlite-ocr's own per-provider health document, the native form of get_health_check_document
pub(crate) struct OcrDocuments;

impl OcrHealthDocuments for OcrDocuments {
    fn health_check_document(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<OcrDocument, HealthProbeError> {
        match get_health_check_document(model, Some(custom_llm_provider)) {
            Ok(document) => Ok(document),
            Err(_error) => Err(HealthProbeError::OcrDocumentUnavailable {
                model: model.to_owned(),
                custom_llm_provider: custom_llm_provider.to_owned(),
            }),
        }
    }
}

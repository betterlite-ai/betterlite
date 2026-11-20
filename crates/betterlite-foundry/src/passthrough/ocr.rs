use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::ocr_provider::passthrough_response as native_passthrough_response;
use betterlite_ocr::response::OCRResponse;

// the native binding litellm.rust_bridge.ocr.entrypoints loads as NATIVE_OCR_PASSTHROUGH_RESPONSE
pub trait NativeOcrPassthrough: Send + Sync {
    fn passthrough_response(
        &self,
        model: &str,
        endpoint: &str,
        body: &[u8],
    ) -> Result<Option<OCRResponse>, OcrError>;
}

// decode_logged_ocr_reply: the selected projector decodes the relayed body, and no projector uses B6808's projection
pub fn decode_logged_ocr_reply(
    projector: Option<&dyn NativeOcrPassthrough>,
    model: &str,
    endpoint: &str,
    body: &[u8],
) -> Result<Option<OCRResponse>, OcrError> {
    projector.map_or_else(
        || native_passthrough_response(model, endpoint, body),
        |selected| selected.passthrough_response(model, endpoint, body),
    )
}

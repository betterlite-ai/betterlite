use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_ocr::json::Object as OcrObject;
use betterlite_ocr::ocr_error::{OcrError, OcrErrorKind, OcrTransportError};
use betterlite_ocr::ocr_input::arguments::consumed_optional_param_names;

use super::SessionCore;
use crate::search_ocr::convert::OcrCall;
use crate::search_ocr::error::NativeFailure;

mod tests;

// project_optional_fields: only the names the provider consumes reach the request
pub(super) fn consumed_params(
    core: &SessionCore,
    call: &OcrCall,
    model: &str,
    provider: &str,
) -> Result<OcrObject, NativeFailure> {
    let names = consumed_optional_param_names(model, Some(provider))
        .map_err(|error| ocr_failure(core, &error, model, provider))?;
    let mut kept = OcrObject::new();
    for (name, value) in &call.optional_params {
        if names.contains(&name.as_str()) {
            let _previous = kept.insert(name.clone(), value.clone());
        }
    }
    Ok(kept)
}

// litellm.exception_type over the OCR failure: the provider's status and body, or the request's own 400
pub(super) fn ocr_failure(
    core: &SessionCore,
    error: &OcrError,
    model: &str,
    provider: &str,
) -> NativeFailure {
    let prefix = format!("{provider}/");
    let unprefixed = model.strip_prefix(&prefix).unwrap_or(model);
    let is_request = matches!(error.kind(), OcrErrorKind::Request);
    let mut raw = RawException {
        model: unprefixed.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: error.to_string(),
        status_code: error.http_status_code(),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: if is_request {
            "ValueError".to_owned()
        } else {
            String::new()
        },
    };
    let mut headers = Vec::new();
    if let OcrError::Provider {
        headers: response_headers,
        body,
        ..
    } = error
    {
        raw.message = Some(body.clone());
        headers.clone_from(response_headers);
    }
    if let OcrError::Transport(OcrTransportError::Http { body, .. }) = error {
        raw.message = Some(body.clone());
    }
    let class = dispatch(&DispatchInput::Raw(Box::new(raw)), &core.redactor);
    NativeFailure::Class {
        class,
        message: error.to_string(),
        headers,
    }
}

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_ocr::json::Object as OcrObject;
use betterlite_ocr::ocr_error::{OcrError, OcrErrorKind, OcrTransportError};
use betterlite_ocr::ocr_input::arguments::consumed_optional_param_names;
use betterlite_ocr::ocr_input::wire::{OcrWireRequest, decode_request_input};
use betterlite_ocr::ocr_route::aocr;
use betterlite_ocr::ocr_settings::OcrSettings;

use super::logging::provider_of;
use super::{SessionCore, encode};
use crate::search_ocr::convert::{OcrAnswer, OcrCall, resolve_document};
use crate::search_ocr::error::{NativeFailure, SessionError};
use crate::search_ocr::ocr_client;

mod tests;

impl SessionCore {
    // litellm.ocr: bind the request, run the provider call and hand back the normalized response
    pub(crate) async fn ocr(
        &self,
        call: OcrCall,
        blobs: &[Vec<u8>],
    ) -> Result<String, NativeFailure> {
        let provider = provider_of(&call.model, call.custom_llm_provider.as_deref()).to_owned();
        let model = call.model.clone();
        let document = resolve_document(&call.document, blobs)
            .map_err(|error| self.ocr_failure(&error, &model, &provider))?;
        let optional_params = self.consumed_params(&call, &model, &provider)?;
        let wire = OcrWireRequest {
            model: call.model,
            document,
            api_key: call.api_key,
            api_base: call.api_base,
            custom_llm_provider: call.custom_llm_provider,
            extra_headers: call.extra_headers,
            optional_params,
            input_sources: call.input_sources,
            timeout_seconds: call.timeout_seconds,
        };
        let request = decode_request_input(wire)
            .map_err(|error| self.ocr_failure(&error, &model, &provider))?;
        let client = ocr_client::build(
            &self.pool,
            &self.tls,
            &self.proxy,
            OcrSettings::from_environment(&self.environment),
            &self.environment,
        )
        .map_err(|source| NativeFailure::Session(SessionError::Client { source }))?;
        match aocr(client, request).await {
            Ok(response) => encode(&OcrAnswer {
                response: &response,
                provider_native_response: response.provider_native_response.as_ref(),
            }),
            Err(error) => Err(self.ocr_failure(&error, &model, &provider)),
        }
    }

    // project_optional_fields: only the names the provider consumes reach the request
    fn consumed_params(
        &self,
        call: &OcrCall,
        model: &str,
        provider: &str,
    ) -> Result<OcrObject, NativeFailure> {
        let names = consumed_optional_param_names(model, Some(provider))
            .map_err(|error| self.ocr_failure(&error, model, provider))?;
        let mut kept = OcrObject::new();
        for (name, value) in &call.optional_params {
            if names.contains(&name.as_str()) {
                let _previous = kept.insert(name.clone(), value.clone());
            }
        }
        Ok(kept)
    }

    // litellm.exception_type over the OCR failure: the provider's status and body, or the request's own 400
    fn ocr_failure(&self, error: &OcrError, model: &str, provider: &str) -> NativeFailure {
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
        } else if let OcrError::Transport(OcrTransportError::Http { body, .. }) = error {
            raw.message = Some(body.clone());
        }
        let class = dispatch(&DispatchInput::Raw(Box::new(raw)), &self.redactor);
        NativeFailure::Class {
            class,
            message: error.to_string(),
            headers,
        }
    }
}

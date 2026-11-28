use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::image_contracts::records::ImageFile;
use betterlite_openai::image_variations::client::{ImageVariationClient, SharedSession};
use betterlite_openai::image_variations::error::OpenAiImageVariationError;
use betterlite_openai::image_variations::logging::{
    ImageVariationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_variations::transport::ImageVariationTransport;
use betterlite_openai::image_variations::{
    ImageVariationParams, ImageVariationRequest, OpenAiImageVariationConfig,
};
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod input;
mod support;
mod tests;

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_oracle_helper_api::error::HelperError;
use http::header::{HeaderName, HeaderValue};
use input::{Function, Input, Probe};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let values = input
            .probes
            .iter()
            .map(|probe| match input.function {
                Function::VariationImagePart => support::transform_value(probe),
                Function::NSizeFormatUser => Ok(support::mapped_value(probe)),
                Function::OmittedDefaultFields
                | Function::ProviderError
                | Function::BinaryFileDescriptor
                | Function::VariationMissingConfigData
                | Function::VariationReplyIdentity => support::request_value(probe),
            })
            .collect::<Result<Vec<Value>, _>>()?;
        Ok(Value::Array(values))
    }
}

pub(super) fn call_for(probe: &Probe) -> ImageVariationRequest {
    ImageVariationRequest {
        model: probe.model.clone(),
        image: ImageFile {
            filename: probe.image_filename.clone(),
            content: probe.image_content.as_bytes().to_vec(),
            content_type: probe.image_content_type.clone(),
        },
        optional_params: probe.optional_params.clone(),
        headers: None,
        api_key: probe.api_key.clone().map(SecretString::from),
        api_base: probe.api_base.clone(),
        organization: probe.organization.clone(),
        timeout: probe
            .timeout
            .map(|seconds| Duration::from_secs(u64::try_from(seconds).unwrap_or_default())),
        params: ImageVariationParams {
            async_call: probe.async_call,
        },
        client: probe.client.map(|kind| ImageVariationClient {
            kind,
            api_key: Some(SecretString::from("sk-supplied")),
            base_url: String::from("https://supplied.test/v1/"),
            timeout: Some(Duration::from_secs(45)),
            max_retries: 1,
            organization: Some(String::from("org-supplied")),
            shared_session: false,
        }),
        shared_session: Some(SharedSession),
        config: (!probe.missing_config).then_some(OpenAiImageVariationConfig),
        boundary: String::from("oracle-boundary"),
        now_seconds: support::NOW,
    }
}

pub(super) struct FakeTransport<'probe> {
    probe: &'probe Probe,
    headers: HeaderMap,
    pub(super) calls: Mutex<Vec<Value>>,
}

impl<'probe> FakeTransport<'probe> {
    pub(super) fn new(probe: &'probe Probe) -> Result<Self, Box<HelperError>> {
        let mut headers = HeaderMap::new();
        for (name, value) in &probe.reply.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| support::invalid(error.to_string()))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|error| support::invalid(error.to_string()))?;
            headers.insert(header_name, header_value);
        }
        Ok(Self {
            probe,
            headers,
            calls: Mutex::default(),
        })
    }
}

fn recorded_parts(body: RequestBody) -> Vec<Value> {
    match body {
        RequestBody::Multipart { parts, .. } => parts
            .into_iter()
            .map(
                |MultipartPart {
                     name,
                     file_name,
                     content_type,
                     data,
                 }| {
                    json!({
                        "name": name,
                        "filename": file_name,
                        "content_type": content_type,
                        "data": String::from_utf8(data).unwrap_or_default(),
                    })
                },
            )
            .collect(),
        RequestBody::Empty | RequestBody::Json(_) | RequestBody::Bytes { .. } => Vec::new(),
    }
}

fn body_of(probe: &Probe) -> Vec<u8> {
    probe
        .reply
        .body_bytes
        .clone()
        .unwrap_or_else(|| probe.reply.body.as_bytes().to_vec())
}

fn recorded_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}

impl ImageVariationTransport for FakeTransport<'_> {
    fn create_variation(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> {
        let parts = recorded_parts(request.body);
        let headers = recorded_headers(&request.headers);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "url": request.url.as_str(),
                "authorization": headers.get("authorization"),
                "organization": headers.get("openai-organization"),
                "parts": parts,
            }));
        let outcome = match self.probe.failure {
            Some(input::Failure::Timeout | input::Failure::Failed) => Err(TransportError::Closed),
            None => Ok(UpstreamResponse {
                status: self.probe.reply.status.unwrap_or(200),
                headers: self.headers.clone(),
                body: body_of(self.probe),
            }),
        };
        ready(outcome)
    }
}

pub(super) struct Recorder;

impl ImageVariationLogger for Recorder {
    fn pre_call(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        Ok(())
    }

    fn post_call(&mut self, _record: PostCallRecord) {}
}

pub(super) fn failure_view(error: &OpenAiImageVariationError) -> Value {
    let diagnostic = if error.message.contains("provider not found") {
        Some("provider")
    } else if error.message.contains("data field is required") {
        Some("data")
    } else {
        None
    };
    json!({
        "mapped": true,
        "status": error.status_code,
        "headers": error.headers.clone().unwrap_or_default(),
        "diagnostic": diagnostic,
    })
}

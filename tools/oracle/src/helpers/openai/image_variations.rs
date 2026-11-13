use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageHeaders};
use betterlite_openai::image_variations::client::{ClientKind, SharedSession, SuppliedClient};
use betterlite_openai::image_variations::error::OpenAiImageVariationError;
use betterlite_openai::image_variations::logging::{
    ImageVariationLogger, PostCallRecord, PreCallRecord, data_view,
};
use betterlite_openai::image_variations::request::{
    begin_image_variation, finish_image_variation, send_image_variation,
};
use betterlite_openai::image_variations::transport::ImageVariationTransport;
use betterlite_openai::image_variations::{
    ImageVariationParams, ImageVariationRequest, OpenAiImageVariationConfig,
};
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::HelperResult;

mod input;
mod support;
mod tests;

use input::{Function, Input, Probe};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        input
            .probes
            .iter()
            .map(|probe| match input.function {
                Function::VariationImagePart => support::transform_value(probe),
                Function::NSizeFormatUser => support::mapped_value(probe),
                Function::OmittedDefaultFields
                | Function::ProviderError
                | Function::BinaryFileDescriptor
                | Function::VariationMissingConfigData
                | Function::VariationReplyIdentity => support::request_value(probe),
            })
            .collect::<Result<Vec<Value>, Box<crate::helpers::HelperError>>>()
            .map(Value::Array)
    }
}

pub(super) fn call_for(probe: &Probe) -> ImageVariationRequest {
    ImageVariationRequest {
        model: probe.model.clone(),
        image: image_for(probe),
        optional_params: probe.optional_params.clone(),
        headers: probe.headers.clone(),
        api_key: probe.api_key.clone().map(SecretString::from),
        api_base: probe.api_base.clone(),
        organization: probe.organization.clone(),
        timeout: probe
            .timeout
            .map(|seconds| Duration::from_secs(u64::try_from(seconds).unwrap_or_default())),
        params: ImageVariationParams {
            async_call: probe.async_call,
        },
        client: probe.client.map(client_kind).map(supplied),
        shared_session: Some(SharedSession),
        config: (!probe.missing_config).then_some(OpenAiImageVariationConfig),
        boundary: String::from("oracle-boundary"),
        now_seconds: support::NOW,
    }
}

const fn client_kind(kind: input::Kind) -> ClientKind {
    match kind {
        input::Kind::Sync => ClientKind::Sync,
        input::Kind::Async => ClientKind::Async,
    }
}

fn supplied(kind: ClientKind) -> SuppliedClient {
    SuppliedClient {
        kind,
        api_key: Some(SecretString::from("sk-supplied")),
        base_url: String::from("https://supplied.test/v1/"),
        timeout: Some(Duration::from_secs(45)),
        max_retries: 1,
        organization: Some(String::from("org-supplied")),
    }
}

fn image_for(probe: &Probe) -> ImageFile {
    ImageFile {
        filename: probe.image_filename.clone(),
        content: probe.image_content.as_bytes().to_vec(),
        content_type: probe.image_content_type.clone(),
    }
}

pub(super) struct FakeTransport<'probe> {
    probe: &'probe Probe,
    headers: HeaderMap,
    calls: Mutex<Vec<Value>>,
}

impl<'probe> FakeTransport<'probe> {
    pub(super) fn new(probe: &'probe Probe) -> Result<Self, Box<crate::helpers::HelperError>> {
        let mut headers = HeaderMap::new();
        for (name, value) in &probe.reply.headers {
            let name = http::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| support::invalid(error.to_string()))?;
            let value = http::header::HeaderValue::from_str(value)
                .map_err(|error| support::invalid(error.to_string()))?;
            headers.insert(name, value);
        }
        Ok(Self {
            probe,
            headers,
            calls: Mutex::default(),
        })
    }
}

impl ImageVariationTransport for FakeTransport<'_> {
    fn create_variation(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> {
        let parts = match request.body {
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
                .collect::<Vec<Value>>(),
            RequestBody::Empty | RequestBody::Json(_) | RequestBody::Bytes { .. } => Vec::new(),
        };
        let headers: BTreeMap<String, String> = request
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
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
                status: self.probe.reply.status,
                headers: self.headers.clone(),
                body: self.probe.reply.body.as_bytes().to_vec(),
            }),
        };
        ready(outcome)
    }
}

#[derive(Default)]
pub(super) struct Recorder {
    pre: Vec<Value>,
    post: Vec<Value>,
}

impl ImageVariationLogger for Recorder {
    fn pre_call(
        &mut self,
        record: PreCallRecord,
    ) -> Result<(), betterlite_logging::logging_pre_post::error::PrePostError> {
        self.pre.push(json!({
            "input": "",
            "api_base": record.api_base,
            "header_names": header_names(record.headers.as_ref()),
            "body": json!(data_view(&record.data)),
            "original_response": Value::Null,
        }));
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(json!({
            "input": Value::Null,
            "api_base": record.api_base,
            "header_names": header_names(record.headers.as_ref()),
            "body": Value::Null,
            "original_response": record.original_response,
        }));
    }
}

fn header_names(headers: Option<&ImageHeaders>) -> Vec<String> {
    let mut names: Vec<String> = headers
        .map(|headers| headers.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

pub(super) fn failure_view(error: &OpenAiImageVariationError) -> Value {
    json!({
        "status": error.status_code,
        "headers": error.headers,
        "diagnostic": diagnostic(&error.message),
    })
}

fn diagnostic(message: &str) -> Option<&'static str> {
    if message.contains("provider not found") {
        return Some("provider");
    }
    if message.contains("data field is required") {
        return Some("data");
    }
    None
}

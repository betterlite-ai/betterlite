use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_generation::error::ImageGenerationFailure;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_generation::request::{
    ImageGenerationOutcome, begin_image_generation, finish_image_generation, send_image_generation,
};
use betterlite_openai::image_generation::transport::ImageGenerationTransport;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::input::{Failure, Probe};
use super::support::{NOW, invalid};
use crate::helpers::HelperResult;

struct FakeTransport {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    failure: Option<Failure>,
    calls: Mutex<Vec<Value>>,
}

impl FakeTransport {
    fn new(probe: &Probe) -> Result<Self, Box<crate::helpers::HelperError>> {
        let headers = probe
            .reply
            .headers
            .iter()
            .map(|(name, value)| {
                let header_name = HeaderName::from_bytes(name.as_bytes())
                    .map_err(|error| invalid(error.to_string()))?;
                let header_value =
                    HeaderValue::from_str(value).map_err(|error| invalid(error.to_string()))?;
                Ok((header_name, header_value))
            })
            .collect::<Result<HeaderMap, Box<crate::helpers::HelperError>>>()?;
        Ok(Self {
            status: probe.reply.status,
            headers,
            body: probe.reply.body.as_bytes().to_vec(),
            failure: probe.failure,
            calls: Mutex::new(Vec::new()),
        })
    }
}

impl ImageGenerationTransport for FakeTransport {
    fn generate(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> {
        let body: Value = match &request.body {
            RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Value::Null),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Value::Null
            }
        };
        let headers: BTreeMap<&str, String> = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let query: BTreeMap<&str, &str> = request
            .query
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "body": body,
                "timeout": request.deadline.as_secs_f64(),
                "url": request.url.as_str(),
                "headers": headers,
                "query": query,
            }));
        let outcome = if self.failure.is_some() {
            Err(TransportError::Closed)
        } else {
            Ok(UpstreamResponse {
                status: self.status,
                headers: self.headers.clone(),
                body: self.body.clone(),
            })
        };
        ready(outcome)
    }
}

// the records the source methods hand their logging object, projected the way the Python recorder is
#[derive(Default)]
pub(crate) struct Recorder {
    pre_calls: Vec<Value>,
    post_calls: Vec<Value>,
}

impl ImageGenerationLogger for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        let mut names: Vec<String> = record.headers.keys().cloned().collect();
        names.sort();
        self.pre_calls.push(json!({
            "input": record.input,
            "additional_args": {
                "api_base": record.api_base.trim_end_matches('/'),
                "complete_input_dict": record.data,
                "acompletion": true,
                "header_names": names,
            },
            "original_response": Value::Null,
        }));
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        let original = match record.original_response {
            Json::Object(fields) => json!(fields),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                Value::Null
            }
        };
        self.post_calls.push(json!({
            "input": record.input,
            "additional_args": {
                "api_base": Value::Null,
                "complete_input_dict": record.data,
                "acompletion": Value::Null,
                "header_names": Vec::<String>::new(),
            },
            "original_response": original,
        }));
    }
}

pub(crate) fn value(probe: &Probe) -> HelperResult {
    let transport = FakeTransport::new(probe)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    let mut recorder = Recorder::default();
    let begun = begin_image_generation(call_for(probe)?, &mut recorder);
    let max_retries = begun.as_ref().ok().map(|prepared| prepared.max_retries);
    let outcome = match begun {
        Ok(prepared) => {
            let sent = runtime.block_on(send_image_generation(&prepared, &transport));
            finish_image_generation(&prepared, sent, &mut recorder)
        }
        Err(failure) => Err(failure),
    };
    let calls = transport
        .calls
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let client = if calls.is_empty() {
        Value::Null
    } else {
        json!({"max_retries": max_retries})
    };
    let (result, provider_headers, error) = outcome_parts(outcome)?;
    Ok(json!({
        "client": client,
        "calls": calls,
        "result": result,
        "provider_headers": provider_headers,
        "error": error,
        "pre_call": recorder.pre_calls.last().cloned().unwrap_or(Value::Null),
        "post_call": recorder.post_calls.last().cloned().unwrap_or(Value::Null),
    }))
}

fn outcome_parts(
    outcome: Result<ImageGenerationOutcome, ImageGenerationFailure>,
) -> Result<(Value, Value, Value), Box<crate::helpers::HelperError>> {
    match outcome {
        Ok(found) => Ok((
            serde_json::to_value(&found.reply).map_err(|error| invalid(error.to_string()))?,
            serde_json::to_value(&found.provider_headers.headers)
                .map_err(|error| invalid(error.to_string()))?,
            Value::Null,
        )),
        Err(failure) => Ok((
            Value::Null,
            Value::Null,
            json!({"mapped": failure.mapped(), "status": failure.status()}),
        )),
    }
}

fn call_for(probe: &Probe) -> Result<ImageGenerationCall, Box<crate::helpers::HelperError>> {
    let seconds = probe
        .timeout
        .ok_or_else(|| invalid(String::from("a request probe needs timeout")))?;
    Ok(ImageGenerationCall {
        model: probe.model.clone(),
        prompt: probe.prompt.clone(),
        optional_params: probe.optional_params.clone(),
        api_key: probe
            .api_key
            .as_ref()
            .map(|key| SecretString::from(key.clone())),
        api_base: probe.api_base.clone(),
        organization: probe.organization.clone(),
        headers: probe.headers.clone(),
        timeout: Duration::from_secs(u64::try_from(seconds).unwrap_or_default()),
        delegate_async: probe.is_async,
        now_seconds: NOW,
    })
}

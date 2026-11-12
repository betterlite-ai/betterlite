use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_generation::request::{
    begin_image_generation, finish_image_generation, send_image_generation,
};
use betterlite_openai::image_generation::transport::ImageGenerationTransport;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::input::{Failure, Probe};
use super::support::{NOW, invalid};
use crate::helpers::HelperResult;

struct Recorded {
    body: Vec<u8>,
    deadline: Duration,
}

struct FakeTransport {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    failure: Option<Failure>,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeTransport {
    fn new(probe: &Probe) -> Result<Self, Box<crate::helpers::HelperError>> {
        let mut headers = HeaderMap::new();
        for (name, value) in &probe.reply.headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| invalid(error.to_string()))?;
            let value = HeaderValue::from_str(value).map_err(|error| invalid(error.to_string()))?;
            headers.insert(name, value);
        }
        Ok(Self {
            status: probe.reply.status,
            headers,
            body: probe.reply.body.as_bytes().to_vec(),
            failure: probe.failure,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<Recorded> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|record| Recorded {
                body: record.body.clone(),
                deadline: record.deadline,
            })
            .collect()
    }
}

impl ImageGenerationTransport for FakeTransport {
    async fn generate(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let body = match &request.body {
            RequestBody::Json(bytes) => bytes.clone(),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                body,
                deadline: request.deadline,
            });
        if self.failure.is_some() {
            return Err(TransportError::Closed);
        }
        Ok(UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        })
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
        }));
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post_calls.push(json!({
            "input": record.input,
            "additional_args": {
                "api_base": Value::Null,
                "complete_input_dict": record.data,
                "acompletion": Value::Null,
                "header_names": Vec::<String>::new(),
            },
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
    let begun = begin_image_generation(call_for(probe), &mut recorder);
    let max_retries = begun.as_ref().ok().map(|prepared| prepared.max_retries);
    let outcome = match begun {
        Ok(prepared) => {
            let sent = runtime.block_on(send_image_generation(&prepared, &transport));
            finish_image_generation(&prepared, sent, &mut recorder)
        }
        Err(failure) => Err(failure),
    };
    let calls = transport.calls();
    let client = if calls.is_empty() {
        Value::Null
    } else {
        json!({
            "api_key": probe.api_key,
            "api_base": probe.api_base,
            "organization": probe.organization,
            "max_retries": max_retries,
        })
    };
    let (result, provider_headers, error) = match outcome {
        Ok(outcome) => (
            serde_json::to_value(&outcome.reply).map_err(|error| invalid(error.to_string()))?,
            serde_json::to_value(&outcome.provider_headers.headers)
                .map_err(|error| invalid(error.to_string()))?,
            Value::Null,
        ),
        Err(failure) => (
            Value::Null,
            Value::Null,
            json!({"mapped": failure.mapped(), "status": failure.status()}),
        ),
    };
    let calls = calls
        .iter()
        .map(|record| {
            let body: betterlite_models::decisions::Json =
                serde_json::from_slice(&record.body).map_err(|error| invalid(error.to_string()))?;
            Ok(json!({"body": body, "timeout": record.deadline.as_secs_f64()}))
        })
        .collect::<Result<Vec<Value>, Box<crate::helpers::HelperError>>>()?;
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

fn call_for(probe: &Probe) -> ImageGenerationCall {
    ImageGenerationCall {
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
        timeout: probe
            .timeout
            .map(|seconds| Duration::from_secs(u64::try_from(seconds).unwrap_or_default())),
        delegate_async: probe.is_async,
        now_seconds: NOW,
    }
}

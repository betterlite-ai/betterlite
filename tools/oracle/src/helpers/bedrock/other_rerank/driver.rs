use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::rerank::error::RerankCallError;
use betterlite_bedrock::rerank::execute::{dispatch_request, resolve_target, transform_response};
use betterlite_bedrock::rerank::plan::{
    LogContext, ParamMap, RerankCall, RerankEnvironment, prepare_request,
};
use betterlite_bedrock::rerank::transport::RerankHttpRequest;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::{LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankRequest, RerankResponse};
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Map, Value, json, value::RawValue};
use thiserror::Error;
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;

use super::fixtures::{Client, DiscardingSink, Source, logging_state, prepost_env};
use super::input::{Function, Probe, Supplied};
use std::str::Utf8Error;
use std::{io, str};

const SIGNING_SECONDS: u64 = 1_700_000_000;
const GENERATED_ID: &str = "00000000-0000-4000-8000-000000000000";

#[derive(Debug, Error)]
pub(super) enum Failure {
    #[error(transparent)]
    Call(#[from] RerankCallError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Redact(#[from] RedactionError),
    #[error("the probe body is not UTF-8")]
    Body(#[from] Utf8Error),
    #[error("the probe needs credentials")]
    MissingCredentials,
    #[error("the probe prepared no request")]
    Shape,
    #[error("the async runtime could not start")]
    Runtime(#[from] io::Error),
}

impl Failure {
    const fn status(&self) -> Option<u16> {
        match self {
            Self::Call(error) => error.status(),
            Self::Json(_)
            | Self::Logging(_)
            | Self::Redact(_)
            | Self::Body(_)
            | Self::MissingCredentials
            | Self::Shape
            | Self::Runtime(_) => None,
        }
    }
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(HelperError {
            class: "RerankCallError".to_owned(),
            status: failure.status(),
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

fn request(probe: &Probe) -> RerankRequest {
    RerankRequest {
        model: probe.model.clone(),
        query: probe.query.clone(),
        top_n: probe.top_n,
        documents: probe.documents.clone(),
        rank_fields: probe.rank_fields.clone(),
        return_documents: probe.return_documents,
        max_chunks_per_doc: probe.max_chunks_per_doc,
        max_tokens_per_doc: None,
        instruction: None,
        truncate_prompt_tokens: None,
        truncation_side: None,
        max_tokens_per_query: None,
    }
}

fn param_map(params: &Map<String, Value>) -> Result<ParamMap, Failure> {
    let mut map = ParamMap::new();
    for (key, value) in params {
        let text = serde_json::to_string(value)?;
        map.insert(key.clone(), RawValue::from_string(text)?);
    }
    Ok(map)
}

struct Harness<'probe> {
    probe: &'probe Probe,
    params: ParamMap,
    extra_headers: Option<Vec<(String, String)>>,
    now: SystemTime,
    source: Source,
    factory: Client,
    supplied: Client,
}

impl<'probe> Harness<'probe> {
    fn new(probe: &'probe Probe) -> Result<Self, Failure> {
        if probe.credentials.access_key_id.is_empty() {
            return Err(Failure::MissingCredentials);
        }
        let now = UNIX_EPOCH
            .checked_add(Duration::from_secs(SIGNING_SECONDS))
            .ok_or(Failure::Shape)?;
        Ok(Self {
            probe,
            params: param_map(&probe.optional_params)?,
            extra_headers: probe
                .extra_headers
                .as_ref()
                .map(|headers| headers.clone().into_iter().collect()),
            now,
            source: Source::new(probe),
            factory: Client::new(&probe.reply),
            supplied: Client::new(&probe.reply),
        })
    }

    fn call<'request>(&'request self, request: &'request RerankRequest) -> RerankCall<'request> {
        RerankCall {
            request,
            optional_params: self.params.clone(),
            api_base: self.probe.api_base.as_deref(),
            extra_headers: self.extra_headers.as_deref(),
            api_key: None,
            timeout: self.probe.timeout_ms.map(Duration::from_millis),
            is_async: self.probe.is_async,
            environment: RerankEnvironment::default(),
            now: self.now,
        }
    }
}

struct Run {
    supplied_seen: Vec<(RerankHttpRequest, bool)>,
    factory_seen: Vec<(RerankHttpRequest, bool)>,
    state: LoggingState,
    // post_call overwrites the pre_call fields in the logging state, as LiteLLM's does, so the snapshot comes from the callback
    captured: Option<Json>,
}

impl Run {
    fn posted(&self) -> Option<&RerankHttpRequest> {
        self.supplied_seen
            .first()
            .or_else(|| self.factory_seen.first())
            .map(|(request, _)| request)
    }
}

fn measure(
    call: &mut RerankCall<'_>,
    harness: &Harness<'_>,
    runtime: &Runtime,
    log: &mut LogContext<'_, '_>,
) -> Result<RerankResponse, Failure> {
    let target = runtime.block_on(resolve_target(call, &harness.source))?;
    let prepared = prepare_request(call, &target, log)?;
    let reply = runtime.block_on(dispatch_request(
        call,
        &prepared,
        Some(SuppliedClient {
            kind: match harness.probe.supplied {
                Supplied::Async => ClientKind::Asynchronous,
                Supplied::Sync => ClientKind::Sync,
            },
            transport: &harness.supplied,
        }),
        &harness.factory,
        log.state,
    ))?;
    transform_response(call, &reply, GENERATED_ID, log).map_err(Failure::from)
}

fn run(probe: &Probe) -> Result<(Run, Result<RerankResponse, Failure>), Failure> {
    let harness = Harness::new(probe)?;
    let mut state = logging_state()?;
    let captured = Mutex::new(None);
    let runtime = Builder::new_current_thread().build()?;
    let request = request(probe);
    let mut call = harness.call(&request);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(Failure::from)?;
    let sink = DiscardingSink;
    let logger = |details: &Json| -> Result<(), PrePostError> {
        let Json::Object(fields) = details else {
            return Ok(());
        };
        if matches!(
            fields.get("log_event_type"),
            Some(Json::Text(event)) if event == "pre_api_call"
        ) {
            *captured.lock().unwrap_or_else(PoisonError::into_inner) = Some(details.clone());
        }
        Ok(())
    };
    let env = prepost_env(&state, &redactor, &sink, Some(&logger));
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let outcome = measure(&mut call, &harness, &runtime, &mut log);
    let run = Run {
        supplied_seen: harness.supplied.requests(),
        factory_seen: harness.factory.requests(),
        state,
        captured: captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    };
    Ok((run, outcome))
}

pub(super) fn observe(probe: &Probe, function: Function) -> Result<Value, Failure> {
    let (run, outcome) = run(probe)?;
    match function {
        Function::ResultsIndexScore => serde_json::to_value(outcome?).map_err(Failure::from),
        Function::RerankError => Ok(error_json(&run, &outcome)),
        Function::RerankTransportLogging => {
            outcome?;
            let request = run.posted().ok_or(Failure::Shape)?;
            let details = &run.state.model_call_details;
            let timeout_ms = request
                .timeout
                .and_then(|timeout| u64::try_from(timeout.as_millis()).ok());
            Ok(json!({
                "posted_url": &request.url,
                "timeout_ms": timeout_ms,
                "pre_call": run.captured.is_some(),
                "post_call": details.get("original_response").is_some(),
            }))
        }
        Function::TextAndJsonDocumentSource
        | Function::RerankModelArn
        | Function::TopN
        | Function::ReturnDocumentContent => prepare_json(&run, outcome),
    }
}

fn prepare_json(run: &Run, outcome: Result<RerankResponse, Failure>) -> Result<Value, Failure> {
    outcome?;
    let request = run.posted().ok_or(Failure::Shape)?;
    Ok(json!({
        "posted_url": &request.url,
        "posted_text": str::from_utf8(&request.body)?,
        "posted_body": serde_json::from_slice::<Value>(&request.body)?,
        "content_type": header(&request.headers, "Content-Type"),
        "signed_headers": {
            "authorization": header(&request.headers, "Authorization").is_some(),
            "amz_date": header(&request.headers, "X-Amz-Date").is_some(),
        },
        "pre_call": pre_call_json(run)?,
    }))
}

fn error_json(run: &Run, outcome: &Result<RerankResponse, Failure>) -> Value {
    json!({
        "ok": outcome.is_ok(),
        "status": outcome.as_ref().err().and_then(Failure::status),
        "post_call": run.state.model_call_details.get("original_response").is_some(),
    })
}

fn pre_call_json(run: &Run) -> Result<Value, Failure> {
    let snapshot = run.captured.as_ref().ok_or(Failure::Shape)?;
    let details = serde_json::to_value(snapshot)?;
    let input = details.get("input").cloned().ok_or(Failure::Shape)?;
    let arguments = details.get("additional_args").ok_or(Failure::Shape)?;
    Ok(json!({
        "input": input,
        "api_base": arguments.get("api_base"),
        "complete_input_dict": arguments.get("complete_input_dict"),
    }))
}

fn header<'headers>(headers: &'headers [(String, String)], name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

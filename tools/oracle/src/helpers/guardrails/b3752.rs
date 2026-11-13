use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::alice::{AliceGuardrail, AliceGuardrailParams, EVALUATE_PATH};
use betterlite_guardrails::conduct::request_payload;
use betterlite_guardrails::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, UnreachableFallback,
};
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

const TIMEOUT_PROBE: Duration = Duration::from_millis(20);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FailureKind {
    Connect,
    Timeout,
}

#[derive(Clone, Debug, Deserialize)]
struct ScriptedAnswer {
    status: u16,
    body: Value,
    raw: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AliceApplyArgs {
    api_key: Option<String>,
    unreachable_fallback: UnreachableFallback,
    texts: Option<Vec<String>>,
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    answer: Option<ScriptedAnswer>,
    failure: Option<FailureKind>,
}

#[derive(Debug, Deserialize)]
struct ConductPayloadArgs {
    texts: Option<Vec<String>>,
    structured_message_text: Option<String>,
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Input {
    AliceApply(Box<AliceApplyArgs>),
    ConductRequestPayload(Box<ConductPayloadArgs>),
}

#[derive(Clone, Debug, serde::Serialize)]
struct Recorded {
    path: String,
    af_api_key: Option<String>,
    content_type: Option<String>,
    body: Value,
}

type Recorder = Arc<Mutex<Vec<Recorded>>>;

enum Endpoint {
    Serving {
        base: String,
        recorder: Recorder,
        _tasks: JoinSet<io::Result<()>>,
    },
    Silent {
        base: String,
        _listener: TcpListener,
    },
    Refused {
        base: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AliceApply(args) => alice_apply(args),
            Input::ConductRequestPayload(args) => conduct_request_payload(args),
        }
    }
}

fn alice_apply(args: &AliceApplyArgs) -> HelperResult {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?
        .block_on(alice_apply_async(args))
}

async fn alice_apply_async(args: &AliceApplyArgs) -> HelperResult {
    let endpoint = start_endpoint(args).await?;
    let timeout = match args.failure {
        Some(FailureKind::Timeout) => TIMEOUT_PROBE,
        Some(FailureKind::Connect) | None => ANSWER_TIMEOUT,
    };
    let guardrail = match AliceGuardrail::new(
        AliceGuardrailParams {
            api_key: args.api_key.clone().map(SecretString::from),
            api_base: Some(endpoint.base().to_owned()),
            unreachable_fallback: Some(args.unreachable_fallback),
        },
        |_name: &str| String::new(),
        build_client()?,
        timeout,
    ) {
        Ok(guardrail) => guardrail,
        Err(error) => {
            return Err(Box::new(HelperError {
                class: "AliceGuardrailMissingSecrets".to_owned(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: error.to_string(),
            }));
        }
    };
    let inputs = GuardrailApiInputs {
        texts: args.texts.clone(),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    let mut request_data = args.request_data.clone();
    let outcome = <AliceGuardrail as GuardrailContract>::apply_guardrail(
        &guardrail,
        inputs,
        &mut request_data,
        args.input_type,
        None,
    )
    .await;
    let (texts, raised, status, blocked_content) = match outcome {
        Ok(inputs) => (inputs.texts.unwrap_or_default(), false, None, None),
        Err(GuardrailFailure::GuardrailRaised {
            status_code,
            blocked_content,
            ..
        }) => (Vec::new(), true, Some(status_code), Some(blocked_content)),
        Err(other) => {
            return Err(Box::new(HelperError {
                class: "GuardrailFailure".to_owned(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: other.to_string(),
            }));
        }
    };
    Ok(json!({
        "texts": texts,
        "requests": endpoint.recorded(),
        "raised": raised,
        "status": status,
        "blocked_content": blocked_content,
    }))
}

fn conduct_request_payload(args: &ConductPayloadArgs) -> HelperResult {
    let inputs = conduct_inputs(args);
    let payload = match request_payload(&inputs, &args.request_data, args.input_type) {
        Some(payload) => json!(serde_json::to_value(payload).or_value_error()?),
        None => Value::Null,
    };
    Ok(json!({ "payload": payload }))
}

fn conduct_inputs(args: &ConductPayloadArgs) -> GuardrailApiInputs {
    let structured_messages = args.structured_message_text.as_ref().map(|text| {
        vec![GuardrailMessage {
            role: "user".to_owned(),
            extra: BTreeMap::from([("content".to_owned(), GuardrailValue::Text(text.clone()))]),
        }]
    });
    GuardrailApiInputs {
        texts: args.texts.clone(),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages,
        model: None,
        stream_holdback_chars: None,
    }
}

async fn start_endpoint(args: &AliceApplyArgs) -> Result<Endpoint, Box<HelperError>> {
    match (&args.answer, &args.failure) {
        (Some(answer), _) => serve(answer.clone()).await,
        (None, Some(FailureKind::Timeout)) => {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            Ok(Endpoint::Silent {
                base: format!("http://{address}"),
                _listener: listener,
            })
        }
        (None, Some(FailureKind::Connect)) => {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            drop(listener);
            Ok(Endpoint::Refused {
                base: format!("http://{address}"),
            })
        }
        (None, None) => Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: "a scripted answer or a failure is required".to_owned(),
        })),
    }
}

async fn serve(answer: ScriptedAnswer) -> Result<Endpoint, Box<HelperError>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let recorder: Recorder = Arc::new(Mutex::new(Vec::new()));
    let handler_recorder = Arc::clone(&recorder);
    let handler = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let recorder = Arc::clone(&handler_recorder);
        let answer = answer.clone();
        async move {
            let entry = Recorded {
                path: uri.path().to_owned(),
                af_api_key: header_text(&headers, "af-api-key"),
                content_type: header_text(&headers, "content-type"),
                body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            };
            match recorder.lock() {
                Ok(mut entries) => entries.push(entry),
                Err(_poisoned) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "recorder poisoned")
                        .into_response();
                }
            }
            answer_response(&answer)
        }
    };
    let router = Router::new().route(EVALUATE_PATH, post(handler));
    let mut tasks = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Ok(Endpoint::Serving {
        base,
        recorder,
        _tasks: tasks,
    })
}

fn answer_response(answer: &ScriptedAnswer) -> Response {
    let status = match StatusCode::from_u16(answer.status) {
        Ok(status) => status,
        Err(_error) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match &answer.raw {
        Some(raw) => (status, raw.clone()).into_response(),
        None => match serde_json::to_vec(&answer.body) {
            Ok(body) => {
                (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
            }
            Err(_error) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn build_client() -> Result<reqwest::Client, Box<HelperError>> {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: "RequestError".to_owned(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: source.to_string(),
            })
        })
}

impl Endpoint {
    fn base(&self) -> &str {
        match self {
            Self::Serving { base, .. } | Self::Silent { base, .. } | Self::Refused { base } => {
                base.as_str()
            }
        }
    }

    fn recorded(&self) -> Vec<Recorded> {
        match self {
            Self::Serving { recorder, .. } => match recorder.lock() {
                Ok(entries) => entries.clone(),
                Err(_poisoned) => Vec::new(),
            },
            Self::Silent { .. } | Self::Refused { .. } => Vec::new(),
        }
    }
}

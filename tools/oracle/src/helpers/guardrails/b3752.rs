use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::alice::{AliceGuardrail, AliceGuardrailParams};
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

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

use endpoint::{Endpoint, build_client, start_endpoint};

mod endpoint;

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
    let guardrail = guardrail_from(args, &endpoint, timeout)?;
    if args.answer.is_none() && args.failure.is_none() {
        return Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: "a scripted answer or a failure is required".to_owned(),
        }));
    }
    let mut request_data = args.request_data.clone();
    let outcome = <AliceGuardrail as GuardrailContract>::apply_guardrail(
        &guardrail,
        state_inputs(args),
        &mut request_data,
        args.input_type,
        None,
    )
    .await;
    let outcome = outcome_of(outcome)?;
    Ok(json!({
        "texts": outcome.texts,
        "requests": endpoint.recorded(),
        "raised": outcome.raised,
        "status": outcome.status,
        "blocked_content": outcome.blocked_content,
    }))
}

struct AliceOutcome {
    texts: Vec<String>,
    raised: bool,
    status: Option<u16>,
    blocked_content: Option<bool>,
}

fn guardrail_from(
    args: &AliceApplyArgs,
    endpoint: &Endpoint,
    timeout: Duration,
) -> Result<AliceGuardrail, Box<HelperError>> {
    AliceGuardrail::new(
        AliceGuardrailParams {
            api_key: args.api_key.clone().map(SecretString::from),
            api_base: Some(endpoint.base().to_owned()),
            unreachable_fallback: Some(args.unreachable_fallback),
        },
        |_name: &str| String::new(),
        build_client()?,
        timeout,
    )
    .map_err(|error| {
        Box::new(HelperError {
            class: "AliceGuardrailMissingSecrets".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })
    })
}

fn state_inputs(args: &AliceApplyArgs) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: args.texts.clone(),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn outcome_of(
    outcome: Result<GuardrailApiInputs, GuardrailFailure>,
) -> Result<AliceOutcome, Box<HelperError>> {
    match outcome {
        Ok(returned) => Ok(AliceOutcome {
            texts: returned.texts.unwrap_or_default(),
            raised: false,
            status: None,
            blocked_content: None,
        }),
        Err(GuardrailFailure::GuardrailRaised {
            status_code,
            blocked_content,
            ..
        }) => Ok(AliceOutcome {
            texts: Vec::new(),
            raised: true,
            status: Some(status_code),
            blocked_content: Some(blocked_content),
        }),
        Err(other) => Err(Box::new(HelperError {
            class: "GuardrailFailure".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: other.to_string(),
        })),
    }
}

fn conduct_request_payload(args: &ConductPayloadArgs) -> HelperResult {
    let inputs = conduct_inputs(args);
    let Some(fields) = request_payload(&inputs, &args.request_data, args.input_type) else {
        return Ok(json!({ "payload": Value::Null }));
    };
    let payload = serde_json::to_value(fields).or_value_error()?;
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

use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::aim::error::AimError;
use betterlite_guardrails::aim::output::AimOutput;
use betterlite_guardrails::aim::settings::{AimHook, SUPPORTED_EVENT_HOOKS};
use betterlite_guardrails::aim::stream::AimStreamChunk;
use futures_util::StreamExt as _;
use reqwest::header::HeaderMap;
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::json;

use super::fakes::{Built, build};
use super::{
    ConstructArgs, HookArg, Input, Op, OutputArgs, RequestArgs, Settings, StreamArgs, failure,
};
use crate::helpers::{Helper, HelperError, HelperResult};

const ANALYZE_PATH: &str = "/fw/v1/analyze";

#[derive(Serialize)]
struct RecordedRequest {
    path_ok: bool,
    headers: BTreeMap<String, String>,
    body: GuardrailValue,
}

#[derive(Serialize)]
struct Rejection {
    status: u16,
    #[serde(rename = "type")]
    error_type: &'static str,
    param: Option<String>,
    code: &'static str,
}

fn prepared(
    settings: &Settings,
    environment: &BTreeMap<String, String>,
    answer: &GuardrailValue,
    status: u16,
    frames: &[GuardrailValue],
) -> Result<(Built, tokio::runtime::Runtime), Box<HelperError>> {
    let built = build(settings, environment, answer, status, frames)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))?;
    Ok((built, runtime))
}

fn recorded(built: &Built) -> Result<RecordedRequest, Box<HelperError>> {
    let posts = built
        .state
        .posts
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(post) = posts.first() else {
        return Err(failure(
            "ValueError",
            "the guardrail sent no request".to_owned(),
        ));
    };
    let mut headers = BTreeMap::new();
    for (name, value) in &post.headers {
        let name = name.as_str();
        if (name.starts_with("x-aim-") || name == "authorization")
            && let Ok(value) = value.to_str()
        {
            headers.insert(name.to_owned(), value.to_owned());
        }
    }
    let body = serde_json::from_slice(&post.body)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    Ok(RecordedRequest {
        path_ok: post.url.ends_with(ANALYZE_PATH),
        headers,
        body,
    })
}

fn rejection(error: &AimError) -> Option<Rejection> {
    match error {
        AimError::Rejected { code, .. } => Some(Rejection {
            status: 400,
            error_type: "invalid_request_error",
            param: None,
            code,
        }),
        _ => None,
    }
}

fn construct(args: &ConstructArgs) -> HelperResult {
    let outcome = build(
        &args.settings,
        &args.environment,
        &GuardrailValue::Null,
        200,
        &[],
    );
    if let Err(error) = &outcome
        && error.class == "AimMissingSecrets"
    {
        return Ok(json!({"missing_secrets": true}));
    }
    let guardrail = outcome?.guardrail;
    let hooks = serde_json::to_value(SUPPORTED_EVENT_HOOKS)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    Ok(json!({
        "api_key": guardrail.api_key.expose_secret(),
        "api_base": guardrail.api_base,
        "ws_api_base": guardrail.ws_api_base,
        "inspect_embeddings": guardrail.inspect_embeddings,
        "supported_event_hooks": hooks,
    }))
}

fn execute(input: &Input) -> HelperResult {
    match input {
        Input::Construct(args) => construct(args),
        Input::Request(args) => {
            let (built, runtime) = prepared(
                &args.settings,
                &args.environment,
                &args.answer,
                args.answer_status,
                &[],
            )?;
            let mut data = args.request_data.clone();
            let hook = match args.hook {
                HookArg::PreCall => AimHook::PreCall,
                HookArg::Moderation => AimHook::Moderation,
            };
            let outcome = runtime.block_on(built.guardrail.call_aim_guardrail(
                &mut data,
                hook,
                args.key_alias.as_deref(),
            ));
            let blocked = match outcome {
                Ok(()) => None,
                Err(error) => match rejection(&error) {
                    Some(blocked) => Some(blocked),
                    None => return Err(failure("AimError", error.to_string())),
                },
            };
            Ok(json!({
                "request": recorded(&built)?,
                "request_data": data,
                "blocked": blocked,
            }))
        }
        Input::Output(args) => {
            let (built, runtime) = prepared(
                &args.settings,
                &args.environment,
                &args.answer,
                args.answer_status,
                &[],
            )?;
            let verdict = runtime.block_on(built.guardrail.call_aim_guardrail_on_output(
                &args.request_data,
                &args.output,
                args.key_alias.as_deref(),
            ));
            let output = match verdict {
                Ok(AimOutput::Blocked { detection_message }) => {
                    json!({"detected": detection_message.is_some(), "redacted_output": null})
                }
                Ok(AimOutput::Redacted { output }) => {
                    json!({"detected": false, "redacted_output": output})
                }
                Ok(AimOutput::Unchanged) => {
                    json!({"detected": false, "redacted_output": args.output})
                }
                Err(error) => match rejection(&error) {
                    Some(blocked) => json!({"detected": false, "rejected": blocked}),
                    None => return Err(failure("AimError", error.to_string())),
                },
            };
            Ok(json!({"request": recorded(&built)?, "output": output}))
        }
        Input::Stream(args) => stream(args),
    }
}

fn stream(args: &StreamArgs) -> HelperResult {
    let (built, runtime) = prepared(
        &args.settings,
        &args.environment,
        &GuardrailValue::Null,
        200,
        &args.frames,
    )?;
    let chunks = args
        .chunks
        .iter()
        .map(|chunk| serde_json::to_string(chunk).map(AimStreamChunk))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| failure("ValueError", error.to_string()))?;
    runtime.block_on(async {
        let stream = built
            .guardrail
            .inspect_output_stream(
                futures_util::stream::iter(chunks),
                &args.request_data,
                args.key_alias.as_deref(),
            )
            .await
            .map_err(|error| failure("AimError", error.to_string()))?;
        let mut stream = Box::pin(stream);
        let mut contents = Vec::new();
        let mut blocked = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => contents.push(
                    chunk
                        .choices
                        .first()
                        .and_then(|choice| choice.delta.content.clone())
                        .unwrap_or_default(),
                ),
                Err(AimError::StreamBlocked { .. }) => blocked = true,
                Err(error) => return Err(failure("AimError", error.to_string())),
            }
        }
        Ok(json!({"chunks": contents, "blocked": blocked}))
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        execute(input)
    }
}

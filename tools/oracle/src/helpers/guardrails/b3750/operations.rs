use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::aim::error::AimError;
use betterlite_guardrails::aim::output::AimOutput;
use betterlite_guardrails::aim::output::call_aim_guardrail_on_output;
use betterlite_guardrails::aim::request::call_aim_guardrail;
use betterlite_guardrails::aim::settings::AimHook;
use betterlite_guardrails::aim::stream::AimStreamChunk;
use betterlite_guardrails::aim::stream::inspect_output_stream;
use futures_util::StreamExt as _;
use serde_json::json;

use super::fakes::{Built, build};
use super::{HookArg, Input, Op, OutputArgs, RequestArgs, Settings, StreamArgs, failure};
use crate::helpers::{Helper, HelperError, HelperResult};

const ANALYZE_PATH: &str = "/fw/v1/analyze";

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

fn recorded(built: &Built) -> HelperResult {
    let Some((url, recorded_headers, bytes)) = built
        .state
        .posts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .map(|post| (post.url.clone(), post.headers.clone(), post.body.clone()))
    else {
        return Err(failure(
            "ValueError",
            "the guardrail sent no request".to_owned(),
        ));
    };
    let mut headers: BTreeMap<String, String> = recorded_headers
        .iter()
        .filter(|(header, _)| {
            header.as_str().starts_with("x-aim-")
                || matches!(header.as_str(), "authorization" | "content-type")
        })
        .filter_map(|(header, value)| {
            Some((header.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect();
    // the real HttpPoster adds this header; the fake poster records the caller's map
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    let body = serde_json::from_slice::<GuardrailValue>(&bytes)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    Ok(json!({
        "path_ok": url.ends_with(ANALYZE_PATH),
        "headers": headers,
        "body": body,
    }))
}

fn rejection(error: &AimError) -> Option<serde_json::Value> {
    let AimError::Rejected { code, .. } = error else {
        return None;
    };
    Some(json!({
        "status": 400,
        "type": "invalid_request_error",
        "param": null,
        "code": code,
    }))
}

fn execute(input: &Input) -> HelperResult {
    match input {
        Input::Request(args) => execute_request(args),
        Input::Output(args) => execute_output(args),
        Input::Stream(args) => stream(args),
    }
}

fn execute_request(args: &RequestArgs) -> HelperResult {
    let (built, runtime) = match prepared(
        &args.settings,
        &args.environment,
        &args.answer,
        args.answer_status,
        &[],
    ) {
        Ok(prepared) => prepared,
        Err(error) if error.class == "AimMissingSecrets" => {
            return Ok(json!({"missing_secrets": true}));
        }
        Err(error) => return Err(error),
    };
    let mut data = args.request_data.clone();
    let hook = match args.hook {
        HookArg::PreCall => AimHook::PreCall,
        HookArg::Moderation => AimHook::Moderation,
    };
    let outcome = runtime.block_on(call_aim_guardrail(
        &built.guardrail,
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

fn execute_output(args: &OutputArgs) -> HelperResult {
    let (built, runtime) = prepared(
        &args.settings,
        &args.environment,
        &args.answer,
        args.answer_status,
        &[],
    )?;
    let verdict = runtime.block_on(call_aim_guardrail_on_output(
        &built.guardrail,
        &args.request_data,
        &args.output,
        args.key_alias.as_deref(),
    ));
    let request = recorded(&built)?;
    match verdict {
        Ok(AimOutput::Blocked { detection_message }) => Ok(json!({
            "request": request,
            "output": {"detected": detection_message.is_some(), "redacted_output": null},
        })),
        Ok(AimOutput::Redacted { output }) => Ok(json!({
            "request": request,
            "output": {"detected": false, "redacted_output": output},
        })),
        Ok(AimOutput::Unchanged) => Ok(json!({
            "request": request,
            "output": {"detected": false, "redacted_output": args.output},
        })),
        Err(error) => match rejection(&error) {
            Some(blocked) => Ok(json!({
                "request": request,
                "output": {"detected": false, "rejected": blocked},
            })),
            None => Err(failure("AimError", error.to_string())),
        },
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
        let stream = inspect_output_stream(
            &built.guardrail,
            futures_util::stream::iter(chunks),
            &args.request_data,
            args.key_alias.as_deref(),
        )
        .await
        .map_err(|error| failure("AimError", error.to_string()))?;
        let mut pinned = Box::pin(stream);
        let mut contents = Vec::new();
        let mut blocked = false;
        while let Some(item) = pinned.next().await {
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

use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::aim::error::AimError;
use betterlite_guardrails::aim::output::AimOutput;
use betterlite_guardrails::aim::settings::{AimHook, SUPPORTED_EVENT_HOOKS};
use betterlite_guardrails::aim::stream::{AimStreamCall, AimStreamChunk};
use futures_util::StreamExt as _;
use reqwest::header::HeaderMap;
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::json;

use super::fakes::{RecordingPoster, build};
use super::{ConstructArgs, HookArg, OutputArgs, RequestArgs, StreamArgs, failure};

#[derive(Serialize)]
struct RecordedRequest {
    path: String,
    headers: BTreeMap<String, String>,
    body: GuardrailValue,
}

#[derive(Serialize)]
struct Rejection {
    status: u16,
    #[serde(rename = "type")]
    error_type: &'static str,
    param: Option<String>,
    code: String,
}

pub(super) fn run_construct(args: &ConstructArgs) -> crate::helpers::HelperResult {
    let built = match build(
        &args.settings,
        &args.environment,
        &GuardrailValue::Null,
        200,
        &[],
    ) {
        Ok(built) => built,
        Err(error) if error.class == "AimMissingSecrets" => {
            return Ok(json!({"missing_secrets": true}));
        }
        Err(error) => return Err(error),
    };
    let guardrail = &built.guardrail;
    Ok(json!({
        "api_key": guardrail.api_key.expose_secret(),
        "api_base": guardrail.api_base,
        "ws_api_base": guardrail.ws_api_base,
        "inspect_embeddings": guardrail.inspect_embeddings,
        "supported_event_hooks": serde_json::to_value(SUPPORTED_EVENT_HOOKS)
            .map_err(|error| failure("ValueError", error.to_string()))?,
    }))
}

fn url_path(url: &str) -> String {
    let after_scheme = url.split_once("://").map_or(url, |(_scheme, rest)| rest);
    after_scheme
        .find('/')
        .map_or("/", |index| &after_scheme[index..])
        .to_owned()
}

fn recorded_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut recorded = BTreeMap::new();
    for (name, value) in headers {
        let name = name.as_str();
        if (name.starts_with("x-aim-") || name == "authorization")
            && let Ok(value) = value.to_str()
        {
            recorded.insert(name.to_owned(), value.to_owned());
        }
    }
    recorded
}

fn recorded_request(
    poster: &RecordingPoster,
) -> Result<RecordedRequest, Box<crate::helpers::HelperError>> {
    let posts = poster.posts.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(post) = posts.first() else {
        return Err(failure(
            "ValueError",
            "the guardrail sent no request".to_owned(),
        ));
    };
    let body = serde_json::from_slice(&post.body)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    Ok(RecordedRequest {
        path: url_path(&post.url),
        headers: recorded_headers(&post.headers),
        body,
    })
}

fn rejection(error: &AimError) -> Option<Rejection> {
    match error {
        AimError::Rejected { code, .. } => Some(Rejection {
            status: 400,
            error_type: "invalid_request_error",
            param: None,
            code: code.as_str().to_owned(),
        }),
        _ => None,
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<crate::helpers::HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))
}

pub(super) fn run_request(args: &RequestArgs) -> crate::helpers::HelperResult {
    let built = build(
        &args.settings,
        &args.environment,
        &args.answer,
        args.answer_status,
        &[],
    )?;
    let hook = match args.hook {
        HookArg::PreCall => AimHook::PreCall,
        HookArg::Moderation => AimHook::Moderation,
    };
    let mut data = args.request_data.clone();
    let runtime = runtime()?;
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
        "request": recorded_request(&built.poster)?,
        "request_data": data,
        "blocked": blocked,
    }))
}

pub(super) fn run_output(args: &OutputArgs) -> crate::helpers::HelperResult {
    let built = build(
        &args.settings,
        &args.environment,
        &args.answer,
        args.answer_status,
        &[],
    )?;
    let runtime = runtime()?;
    let outcome = runtime.block_on(built.guardrail.call_aim_guardrail_on_output(
        &args.request_data,
        &args.output,
        args.key_alias.as_deref(),
    ));
    let output = match outcome {
        Ok(AimOutput::Unchanged) => json!({"detected": false, "redacted_output": args.output}),
        Ok(AimOutput::Redacted { output }) => {
            json!({"detected": false, "redacted_output": output})
        }
        Ok(AimOutput::Blocked { detection_message }) => {
            json!({"detected": detection_message.is_some(), "redacted_output": null})
        }
        Err(error) => match rejection(&error) {
            Some(blocked) => json!({"detected": false, "rejected": blocked}),
            None => return Err(failure("AimError", error.to_string())),
        },
    };
    Ok(json!({
        "request": recorded_request(&built.poster)?,
        "output": output,
    }))
}

pub(super) fn run_stream(args: &StreamArgs) -> crate::helpers::HelperResult {
    let built = build(
        &args.settings,
        &args.environment,
        &GuardrailValue::Null,
        200,
        &args.frames,
    )?;
    let mut chunks = Vec::new();
    for chunk in &args.chunks {
        let text = serde_json::to_string(chunk)
            .map_err(|error| failure("ValueError", error.to_string()))?;
        chunks.push(AimStreamChunk(text));
    }
    let runtime = runtime()?;
    let collected = runtime.block_on(async {
        let call = AimStreamCall {
            request_data: &args.request_data,
            key_alias: args.key_alias.as_deref(),
        };
        let stream = built
            .guardrail
            .inspect_output_stream(futures_util::stream::iter(chunks), call)
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
    })?;
    Ok(json!({"stream": collected}))
}

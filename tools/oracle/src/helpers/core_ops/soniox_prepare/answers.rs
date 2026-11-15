use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::soniox::prepare::body::build_create_body;
use betterlite_core::api::soniox::prepare::decode::decode_soniox_reply;
use betterlite_core::api::soniox::prepare::error::PrepareError;
use betterlite_core::api::soniox::prepare::logging::{
    SonioxEvent, SonioxLogger, SonioxPostCall, SonioxPreCall, log_post_call, log_pre_call,
    redacted_body,
};
use betterlite_core::api::soniox::prepare::raise::raise_for_response;
use betterlite_core::api::soniox::prepare::{PrepareInput, Prepared, prepare};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::api::audio_file_input::AudioFile;
use betterlite_openai::openai_base_config::{Headers, Object};
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Value, json};

use super::input::{Call, PrepareArgs};
use crate::helpers::{HelperError, HelperResult};

type Failure = Box<HelperError>;

struct Recorder {
    failing: bool,
    recorded: Option<Value>,
}

impl Recorder {
    fn refuse_when_failing(&self) -> Result<(), PrePostError> {
        if self.failing {
            return Err(PrePostError::CallbackFailed {
                message: String::from("callback boom"),
            });
        }
        Ok(())
    }
}

impl SonioxLogger for Recorder {
    fn record(&mut self, event: SonioxEvent) -> Result<(), PrePostError> {
        self.refuse_when_failing()?;
        self.recorded = Some(match event {
            SonioxEvent::PreCall(SonioxPreCall { url, body }) => json!({
                "input": Value::Null,
                "additional_args": {
                    "api_base": url,
                    "atranscription": true,
                    "complete_input_dict": body,
                },
            }),
            SonioxEvent::PostCall(SonioxPostCall {
                input,
                original_response,
                body,
            }) => json!({
                "input": input,
                "additional_args": { "complete_input_dict": body },
                "original_response": original_response,
            }),
        });
        Ok(())
    }
}

pub(super) fn answer(call: &Call) -> HelperResult {
    match call {
        Call::Prepare(args) => prepare_answer(args),
        Call::CreateBody {
            prepare,
            model,
            file_id,
        } => Ok(create_body_answer(prepare, model, file_id.as_deref())),
        Call::Redact { body } => {
            let redacted = redacted_body(body);
            Ok(json!({ "redacted": redacted, "body": Value::Object(body.clone()) }))
        }
        Call::Raise {
            status,
            action,
            body,
            headers,
            probes,
        } => raise_answer(*status, action, body, headers, probes),
        Call::PreCall {
            api_base,
            body,
            failing,
        } => Ok(pre_call_answer(api_base, body, *failing)),
        Call::PostCall {
            audio,
            original_response,
            body,
            failing,
        } => Ok(post_call_answer(
            audio.as_deref(),
            original_response,
            body,
            *failing,
        )),
    }
}

fn prepare_answer(args: &PrepareArgs) -> HelperResult {
    match resolve(args) {
        Ok(prepared) => encode_prepared(args, &prepared),
        Err(error) => Ok(raised(&error)),
    }
}

fn create_body_answer(args: &PrepareArgs, model: &str, file_id: Option<&str>) -> Value {
    match resolve(args) {
        Ok(prepared) => {
            let body = build_create_body(model, &prepared.params, &prepared.options, file_id);
            json!({ "ok": true, "body": Value::Object(body) })
        }
        Err(error) => raised(&error),
    }
}

fn resolve(args: &PrepareArgs) -> Result<Prepared, PrepareError> {
    prepare(&PrepareInput {
        optional_params: &args.optional_params,
        headers: args.headers.clone(),
        api_key: args.api_key.as_deref(),
        api_base: args.api_base.as_deref(),
        environment: &args.environment,
    })
}

fn encode_prepared(args: &PrepareArgs, prepared: &Prepared) -> HelperResult {
    let poll_interval = prepared
        .options
        .poll_interval
        .to_f64()
        .ok_or_else(|| failure("the poll interval is not a finite number"))?;
    let params = Value::Object(prepared.params.clone());
    let options = json!({
        "poll_interval": poll_interval,
        "max_attempts": prepared.options.max_attempts,
        "cleanup": prepared.options.cleanup,
        "filename_override": prepared.options.filename_override,
        "audio_url": prepared.options.audio_url,
        "file_id": prepared.options.file_id,
        "response_format": prepared.options.response_format,
    });
    Ok(json!({
        "ok": true,
        "auth_headers": prepared.auth_headers,
        "api_base": prepared.api_base,
        "params": params,
        "options": options,
        "input": args.optional_params,
    }))
}

fn raise_answer(
    status: i64,
    action: &str,
    body: &str,
    headers: &Headers,
    probes: &[String],
) -> HelperResult {
    let reply = decode_soniox_reply(
        status,
        headers.clone(),
        body.as_bytes(),
        &ApiLimits::default(),
    )
    .map_err(failure)?;
    let Err(exception) = raise_for_response(&reply, action) else {
        return Ok(json!({ "ok": true }));
    };
    let watched: BTreeMap<String, Option<String>> = headers
        .keys()
        .map(|name| {
            let value = exception
                .headers
                .as_ref()
                .and_then(|given| given.get(name))
                .cloned();
            (name.to_lowercase(), value)
        })
        .collect();
    let found: BTreeMap<String, bool> = probes
        .iter()
        .map(|probe| (probe.clone(), exception.message.contains(probe.as_str())))
        .collect();
    Ok(json!({
        "raised": exception.status_code,
        "headers": watched,
        "probes": found,
    }))
}

fn pre_call_answer(api_base: &str, body: &Object, failing: bool) -> Value {
    let mut recorder = Recorder {
        failing,
        recorded: None,
    };
    log_pre_call(&mut recorder, api_base, body);
    json!({ "recorded": recorder.recorded, "body": body })
}

fn post_call_answer(
    audio: Option<&str>,
    original_response: &Object,
    body: &Object,
    failing: bool,
) -> Value {
    let mut recorder = Recorder {
        failing,
        recorded: None,
    };
    let file = audio.map(AudioFile::Text);
    log_post_call(&mut recorder, file.as_ref(), original_response, body);
    json!({ "recorded": recorder.recorded, "body": body })
}

fn raised(error: &PrepareError) -> Value {
    match error {
        PrepareError::Auth(exception) => json!({ "raised": exception.status_code }),
        PrepareError::Interval { .. }
        | PrepareError::Attempts { .. }
        | PrepareError::Text { .. }
        | PrepareError::Cleanup { .. } => json!({ "raised": Value::Null }),
    }
}

fn failure<Error>(error: Error) -> Failure
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

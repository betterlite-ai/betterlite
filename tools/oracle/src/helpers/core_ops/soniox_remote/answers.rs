use std::time::Duration;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::soniox::prepare::Prepared;
use betterlite_core::api::soniox::prepare::options::HandlerOptions;
use betterlite_core::api::soniox::prepare::response::TranscriptionMeta;
use betterlite_core::api::soniox::records::error::SonioxException;
use betterlite_core::api::soniox::remote::SonioxRemote;
use betterlite_core::api::soniox::remote::error::RemoteError;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::{Headers, Object};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::input::{Call, CleanupArgs, PollArgs, UploadArgs};
use super::recorder::{Failure, Recorder, failure};
use betterlite_oracle_helper_api::helper::HelperResult;

const BOUNDARY: &str = "soniox-remote-oracle";

pub(super) fn answer(runtime: &Runtime, call: &Call) -> HelperResult {
    match call {
        Call::Upload(fields) => upload(runtime, fields),
        Call::Poll(fields) => poll(runtime, fields),
        Call::Cleanup(fields) => cleanup(runtime, fields),
    }
}

fn upload(runtime: &Runtime, fields: &UploadArgs) -> HelperResult {
    let sync = upload_once(runtime, fields)?;
    let asynchronous = upload_once(runtime, fields)?;
    Ok(json!({ "sync": sync, "async": asynchronous }))
}

fn poll(runtime: &Runtime, fields: &PollArgs) -> HelperResult {
    let sync = poll_once(runtime, fields)?;
    let asynchronous = poll_once(runtime, fields)?;
    Ok(json!({ "sync": sync, "async": asynchronous }))
}

fn cleanup(runtime: &Runtime, fields: &CleanupArgs) -> HelperResult {
    let sync = cleanup_once(runtime, fields)?;
    let asynchronous = cleanup_once(runtime, fields)?;
    Ok(json!({ "sync": sync, "async": asynchronous }))
}

fn upload_once(runtime: &Runtime, fields: &UploadArgs) -> HelperResult {
    let recorder = Recorder::new(&fields.answers, &fields.auth_headers)?;
    let audio = ProcessedAudioFile {
        file_content: fields.content.as_bytes().to_vec(),
        filename: fields.filename.clone(),
        content_type: fields.content_type.clone(),
    };
    let ready = prepared(
        &fields.auth_headers,
        &fields.api_base,
        upload_options(fields),
    );
    let remote = remote(&recorder, &ready, fields.timeout_ms);
    let outcome = runtime.block_on(remote.upload(&audio));
    Ok(view(&recorder, upload_outcome(outcome)))
}

fn poll_once(runtime: &Runtime, fields: &PollArgs) -> HelperResult {
    let recorder = Recorder::new(&fields.answers, &fields.auth_headers)?;
    let ready = prepared(
        &fields.auth_headers,
        &fields.api_base,
        poll_options(fields)?,
    );
    let remote = remote(&recorder, &ready, fields.timeout_ms);
    let outcome = runtime.block_on(remote.poll(&fields.transcription_id));
    Ok(view(&recorder, poll_outcome(outcome)))
}

fn cleanup_once(runtime: &Runtime, fields: &CleanupArgs) -> HelperResult {
    let recorder = Recorder::new(&fields.answers, &fields.auth_headers)?;
    let ready = prepared(
        &fields.auth_headers,
        &fields.api_base,
        cleanup_options(fields),
    );
    let remote = remote(&recorder, &ready, fields.timeout_ms);
    runtime.block_on(remote.cleanup(
        fields.transcription_id.as_deref(),
        fields.file_id.as_deref(),
    ));
    Ok(view(
        &recorder,
        json!({ "kind": "returned", "value": Value::Null }),
    ))
}

fn upload_options(fields: &UploadArgs) -> HandlerOptions {
    HandlerOptions {
        poll_interval: Decimal::ONE,
        max_attempts: 1,
        cleanup: Vec::new(),
        filename_override: fields.filename_override.clone(),
        audio_url: None,
        file_id: None,
        response_format: None,
    }
}

fn poll_options(fields: &PollArgs) -> Result<HandlerOptions, Failure> {
    let poll_interval = Decimal::from_f64(fields.poll_interval)
        .ok_or_else(|| failure("the poll interval is not a finite number"))?;
    Ok(HandlerOptions {
        poll_interval,
        max_attempts: fields.max_attempts,
        cleanup: Vec::new(),
        filename_override: None,
        audio_url: None,
        file_id: None,
        response_format: None,
    })
}

fn cleanup_options(fields: &CleanupArgs) -> HandlerOptions {
    HandlerOptions {
        poll_interval: Decimal::ONE,
        max_attempts: 1,
        cleanup: fields.cleanup.clone(),
        filename_override: None,
        audio_url: None,
        file_id: None,
        response_format: None,
    }
}

fn prepared(auth_headers: &Headers, api_base: &str, options: HandlerOptions) -> Prepared {
    Prepared {
        auth_headers: auth_headers.clone(),
        api_base: api_base.to_owned(),
        params: Object::new(),
        options,
    }
}

fn remote<'call>(
    recorder: &'call Recorder,
    prepared: &'call Prepared,
    timeout_ms: u64,
) -> SonioxRemote<'call, Recorder, Recorder> {
    SonioxRemote {
        transport: recorder,
        sleeper: recorder,
        prepared,
        timeout: Duration::from_millis(timeout_ms),
        limits: ApiLimits::default(),
        boundaries: || BOUNDARY.to_owned(),
    }
}

fn view(recorder: &Recorder, outcome: Value) -> Value {
    json!({
        "requests": recorder.calls(),
        "sleeps": recorder.sleeps(),
        "outcome": outcome,
    })
}

fn upload_outcome(outcome: Result<String, RemoteError>) -> Value {
    match outcome {
        Ok(uploaded) => json!({ "kind": "returned", "value": uploaded }),
        Err(error) => failure_outcome(error),
    }
}

fn poll_outcome(outcome: Result<TranscriptionMeta, RemoteError>) -> Value {
    match outcome {
        Ok(meta) => json!({ "kind": "returned", "value": Value::Object(meta.raw) }),
        Err(error) => failure_outcome(error),
    }
}

fn failure_outcome(error: RemoteError) -> Value {
    match error {
        RemoteError::Provider(provider) => provider_outcome(&provider),
        RemoteError::Transport(_transport) => json!({ "kind": "transport" }),
        refused @ (RemoteError::Reply(_)
        | RemoteError::Url { .. }
        | RemoteError::HeaderName { .. }
        | RemoteError::HeaderValue { .. }
        | RemoteError::MissingHeader { .. }
        | RemoteError::Interval { .. }) => {
            json!({ "kind": "reply", "message": refused.to_string() })
        }
    }
}

fn provider_outcome(provider: &SonioxException) -> Value {
    json!({
        "kind": "provider",
        "status": provider.status_code,
        "message": provider.message,
        "headers": provider.headers.clone(),
    })
}

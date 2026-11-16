use std::time::Duration;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::soniox::prepare::Prepared;
use betterlite_core::api::soniox::prepare::options::HandlerOptions;
use betterlite_core::api::soniox::prepare::response::TranscriptionMeta;
use betterlite_core::api::soniox::records::error::SonioxException;
use betterlite_core::api::soniox::remote::SonioxRemote;
use betterlite_core::api::soniox::remote::cleanup::clean_up;
use betterlite_core::api::soniox::remote::error::RemoteError;
use betterlite_core::api::soniox::remote::poll::poll_until_completed;
use betterlite_core::api::soniox::remote::upload::upload_file;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::{Headers, Object};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::input::{Answer, Call, CleanupArgs, PollArgs, UploadArgs};
use super::recorder::{Failure, Recorder, failure};
use betterlite_oracle_helper_api::helper::HelperResult;

const BOUNDARY: &str = "soniox-remote-oracle";

struct Choices {
    poll_interval: Decimal,
    max_attempts: i64,
    cleanup: Vec<String>,
    filename_override: Option<String>,
}

// Every call carries the authenticated headers, the endpoint and the scripted answers the recorder replays
trait Shared {
    fn shared(&self) -> (&[Answer], &Headers, &str);
}

impl Shared for UploadArgs {
    fn shared(&self) -> (&[Answer], &Headers, &str) {
        (
            self.answers.as_slice(),
            &self.auth_headers,
            self.api_base.as_str(),
        )
    }
}

impl Shared for PollArgs {
    fn shared(&self) -> (&[Answer], &Headers, &str) {
        (
            self.answers.as_slice(),
            &self.auth_headers,
            self.api_base.as_str(),
        )
    }
}

impl Shared for CleanupArgs {
    fn shared(&self) -> (&[Answer], &Headers, &str) {
        (
            self.answers.as_slice(),
            &self.auth_headers,
            self.api_base.as_str(),
        )
    }
}

// LiteLLM's sync and async helpers share this behavior, so both runs answer the same way
pub(super) fn answer(runtime: &Runtime, call: &Call) -> HelperResult {
    let sync = answer_once(runtime, call)?;
    let asynchronous = answer_once(runtime, call)?;
    Ok(json!({ "sync": sync, "async": asynchronous }))
}

fn answer_once(runtime: &Runtime, call: &Call) -> HelperResult {
    match call {
        Call::Upload(fields) => upload(runtime, fields),
        Call::Poll(fields) => poll(runtime, fields),
        Call::Cleanup(fields) => cleanup(runtime, fields),
    }
}

fn upload(runtime: &Runtime, fields: &UploadArgs) -> HelperResult {
    let (recorder, ready) = setup(fields, upload_options(fields))?;
    let remote = remote_for(&recorder, &ready, fields.timeout_ms);
    let outcome = runtime.block_on(upload_file(&remote, &audio(fields)));
    Ok(view(&recorder, upload_outcome(outcome)))
}

fn poll(runtime: &Runtime, fields: &PollArgs) -> HelperResult {
    let (recorder, ready) = setup(fields, poll_options(fields)?)?;
    let remote = remote_for(&recorder, &ready, fields.timeout_ms);
    let outcome = runtime.block_on(poll_until_completed(&remote, &fields.transcription_id));
    Ok(view(&recorder, poll_outcome(outcome)))
}

fn cleanup(runtime: &Runtime, fields: &CleanupArgs) -> HelperResult {
    let (recorder, ready) = setup(fields, cleanup_options(fields))?;
    let remote = remote_for(&recorder, &ready, fields.timeout_ms);
    let transcription = fields.transcription_id.as_deref();
    let file = fields.file_id.as_deref();
    runtime.block_on(clean_up(&remote, transcription, file));
    Ok(view(
        &recorder,
        json!({ "kind": "returned", "value": null }),
    ))
}

fn setup(fields: &dyn Shared, options: HandlerOptions) -> Result<(Recorder, Prepared), Failure> {
    let (answers, auth_headers, api_base) = fields.shared();
    let recorder = Recorder::new(answers, auth_headers)?;
    Ok((recorder, prepared(auth_headers, api_base, options)))
}

fn upload_options(fields: &UploadArgs) -> HandlerOptions {
    options(Choices {
        poll_interval: Decimal::ONE,
        max_attempts: 1,
        cleanup: Vec::new(),
        filename_override: fields.filename_override.clone(),
    })
}

fn cleanup_options(fields: &CleanupArgs) -> HandlerOptions {
    options(Choices {
        poll_interval: Decimal::ONE,
        max_attempts: 1,
        cleanup: fields.cleanup.clone(),
        filename_override: None,
    })
}

fn poll_options(fields: &PollArgs) -> Result<HandlerOptions, Failure> {
    let poll_interval = Decimal::from_f64(fields.poll_interval)
        .ok_or_else(|| failure("the poll interval is not a finite number"))?;
    Ok(options(Choices {
        poll_interval,
        max_attempts: fields.max_attempts,
        cleanup: Vec::new(),
        filename_override: None,
    }))
}

fn options(choices: Choices) -> HandlerOptions {
    HandlerOptions {
        poll_interval: choices.poll_interval,
        max_attempts: choices.max_attempts,
        cleanup: choices.cleanup,
        filename_override: choices.filename_override,
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

fn audio(fields: &UploadArgs) -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: fields.content.as_bytes().to_vec(),
        filename: fields.filename.clone(),
        content_type: fields.content_type.clone(),
    }
}

fn remote_for<'call>(
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

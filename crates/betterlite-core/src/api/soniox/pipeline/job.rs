use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AuthorizedAudioFiles, ProcessedAudioFile, process_audio_file,
};
use betterlite_openai::openai_base_config::Object;

use super::PipelineCall;
use super::SonioxPipeline;
use super::audio;
use super::error::PipelineError;
use super::hidden;
use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::prepare::Prepared;
use crate::api::soniox::prepare::body::build_create_body;
use crate::api::soniox::prepare::logging::{
    SonioxEvent, SonioxLogger, log_post_call, log_pre_call,
};
use crate::api::soniox::records::error::SonioxException;
use crate::api::soniox::records::response::{
    SonioxTranscriptPayload, SonioxTranscriptionResponse, build_response_from_payload,
};
use crate::api::soniox::remote::SonioxRemote;
use crate::api::soniox::remote::cleanup::clean_up;
use crate::api::soniox::remote::poll::poll_until_completed;
use crate::api::soniox::remote::transport::{RemoteTransport, Sleeper};
use crate::api::soniox::remote::upload::upload_file;

pub const MISSING_INPUT: &str = "Soniox transcription requires one of: a file argument, an `audio_url` kwarg, or a `file_id` kwarg.";
pub const MISSING_INPUT_STATUS: i64 = 400;

// The supplied response, audio and logger are updated in place, as LiteLLM's handler mutates its arguments
pub struct PipelineTarget<'call, 'file> {
    pub audio_file: Option<&'call mut AudioFile<'file>>,
    pub response: &'call mut SonioxTranscriptionResponse,
    pub logger: &'call mut (dyn SonioxLogger + Send),
}

// Every id is invocation-local, so a failed call can only clean up what this invocation learned
pub(super) struct JobState {
    selected_file: Option<String>,
    uploaded_file: Option<String>,
    transcription: Option<String>,
}

// Everything the network half of the job needs: the borrows it uses and the work already done
pub(super) struct Started<'run, T, S> {
    prepared: Prepared,
    transport: &'run T,
    sleeper: &'run S,
    timeout: Duration,
    limits: ApiLimits,
    boundaries: fn() -> String,
    processed: Option<ProcessedAudioFile>,
    logged_audio: Option<String>,
    state: JobState,
    response: &'run mut SonioxTranscriptionResponse,
    logger: &'run mut (dyn SonioxLogger + Send),
    model: &'run str,
}

// LiteLLM reads and uploads the audio before it enters the try, so a start that fails owns no id yet
pub(super) fn begin<'run, P, S, T>(
    pipeline: &'run SonioxPipeline<'_, P, S>,
    call: &'run PipelineCall<'_>,
    prepared: Prepared,
    target: PipelineTarget<'run, '_>,
    transport: &'run T,
) -> Result<Started<'run, T, S>, PipelineError>
where
    P: RemoteTransport,
    S: Sleeper,
    T: RemoteTransport,
{
    let PipelineTarget {
        mut audio_file,
        response,
        logger,
    } = target;
    let state = JobState {
        selected_file: prepared.options.file_id.clone().filter(|id| !id.is_empty()),
        uploaded_file: None,
        transcription: None,
    };
    let processed = select_reference(&prepared, pipeline.audio_files, audio_file.as_deref_mut())?;
    let logged_audio = logged_audio(audio_file.as_deref());
    Ok(Started {
        prepared,
        transport,
        sleeper: pipeline.sleeper,
        timeout: call.timeout,
        limits: pipeline.limits,
        boundaries: pipeline.boundaries,
        processed,
        logged_audio,
        state,
        response,
        logger,
        model: call.model,
    })
}

// The per-call borrows the steps below share, grouped so run stays under the argument limit
pub(super) struct JobRun<'job> {
    state: &'job mut JobState,
    logged_audio: Option<&'job str>,
    model: &'job str,
    response: &'job mut SonioxTranscriptionResponse,
    logger: &'job mut (dyn SonioxLogger + Send),
}

pub(super) async fn finish<T, S>(started: Started<'_, T, S>) -> Result<(), PipelineError>
where
    T: RemoteTransport,
    S: Sleeper,
{
    let Started {
        prepared,
        transport,
        sleeper,
        timeout,
        limits,
        boundaries,
        processed,
        logged_audio,
        mut state,
        response,
        logger,
        model,
    } = started;
    let remote = SonioxRemote {
        transport,
        sleeper,
        prepared: &prepared,
        timeout,
        limits,
        boundaries,
    };
    let targets = JobRun {
        state: &mut state,
        logged_audio: logged_audio.as_deref(),
        model,
        response,
        logger,
    };
    let outcome = run(&remote, processed, targets).await;
    clean_up(
        &remote,
        state.transcription.as_deref(),
        state.uploaded_file.as_deref(),
    )
    .await;
    outcome
}

async fn run<T, S>(
    remote: &SonioxRemote<'_, T, S>,
    processed: Option<ProcessedAudioFile>,
    JobRun {
        state,
        logged_audio,
        model,
        response,
        logger,
    }: JobRun<'_>,
) -> Result<(), PipelineError>
where
    T: RemoteTransport,
    S: Sleeper,
{
    if let Some(audio) = processed {
        let uploaded = upload_file(remote, &audio).await?;
        state.uploaded_file = Some(uploaded.clone());
        state.selected_file = Some(uploaded);
    }
    let body = build_create_body(
        model,
        &remote.prepared.params,
        &remote.prepared.options,
        state.selected_file.as_deref(),
    );
    log_pre_call(&mut *logger, &remote.prepared.api_base, &body);
    let created = remote.create(&body).await?;
    state.transcription = Some(created.id.clone());
    let meta = poll_until_completed(remote, &created.id).await?;
    let transcript = remote.fetch(&created.id).await?;
    let payload = SonioxTranscriptPayload {
        transcription: meta.raw,
        transcript,
    };
    let format = remote.prepared.options.response_format.as_deref();
    let updated = build_response_from_payload(&payload, Some(response.clone()), format)?;
    *response = updated;
    let audio = logged_audio.map(|name| AudioFile::Unsupported { python_class: name });
    log_post_call(
        &mut *logger,
        audio.as_ref(),
        &original_response(&payload),
        &body,
    );
    hidden::apply(response, model, meta.audio_duration_ms.as_ref())?;
    Ok(())
}

// LiteLLM uploads only without a file_id or audio_url; only a supplied file can be missing.
fn select_reference(
    prepared: &Prepared,
    sources: &(dyn AuthorizedAudioFiles + Sync),
    audio_file: Option<&mut AudioFile<'_>>,
) -> Result<Option<ProcessedAudioFile>, PipelineError> {
    let audio_url = prepared.options.audio_url.as_deref();
    let has_url = audio_url.is_some_and(|url| !url.is_empty());
    let file_id = prepared.options.file_id.as_deref();
    if file_id.is_some_and(|id| !id.is_empty()) || has_url {
        return Ok(None);
    }
    let Some(selected) = audio_file else {
        return Err(missing_input());
    };
    let processed = process_audio_file(audio::reborrow(selected), sources)?;
    Ok(Some(processed))
}

// The audio's borrow cannot cross an await, so post_call's input is captured and replayed.
struct LoggedAudio(Option<String>);

impl SonioxLogger for LoggedAudio {
    fn record(&mut self, event: SonioxEvent) -> Result<(), PrePostError> {
        if let SonioxEvent::PostCall(record) = event
            && let Json::Text(name) = record.input
        {
            self.0 = Some(name);
        }
        Ok(())
    }
}

fn logged_audio(audio: Option<&AudioFile<'_>>) -> Option<String> {
    let mut captured = LoggedAudio(None);
    log_post_call(&mut captured, audio, &Object::new(), &Object::new());
    captured.0
}

// LiteLLM hands post_call the same two-object payload that builds the response
fn original_response(payload: &SonioxTranscriptPayload) -> Object {
    let mut original = Object::new();
    original.insert(
        "transcription".to_owned(),
        JsonValue::Object(payload.transcription.clone()),
    );
    original.insert(
        "transcript".to_owned(),
        JsonValue::Object(payload.transcript.clone()),
    );
    original
}

fn missing_input() -> PipelineError {
    PipelineError::Provider(SonioxException {
        message: String::from(MISSING_INPUT),
        status_code: MISSING_INPUT_STATUS,
        headers: None,
    })
}

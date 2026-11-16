use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AuthorizedAudioFiles, process_audio_file,
};
use betterlite_openai::openai_base_config::Object;

use super::PipelineCall;
use super::SonioxPipeline;
use super::audio;
use super::error::PipelineError;
use super::exchange;
use super::hidden;
use crate::api::soniox::prepare::Prepared;
use crate::api::soniox::prepare::body::build_create_body;
use crate::api::soniox::prepare::logging::{SonioxLogger, log_post_call, log_pre_call};
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
    pub logger: &'call mut dyn SonioxLogger,
}

// Every id is invocation-local, so a failed call can only clean up what this invocation learned
pub(super) struct JobState {
    selected_file: Option<String>,
    uploaded_file: Option<String>,
    transcription: Option<String>,
}

impl JobState {
    fn new(prepared: &Prepared) -> Self {
        Self {
            selected_file: prepared.options.file_id.clone().filter(|id| !id.is_empty()),
            uploaded_file: None,
            transcription: None,
        }
    }
}

pub(super) async fn execute<P, S, T>(
    pipeline: &SonioxPipeline<'_, P, S>,
    call: &PipelineCall<'_>,
    prepared: &Prepared,
    target: PipelineTarget<'_, '_>,
    transport: &T,
) -> Result<(), PipelineError>
where
    P: RemoteTransport,
    S: Sleeper,
    T: RemoteTransport,
{
    let remote = SonioxRemote {
        transport,
        sleeper: pipeline.sleeper,
        prepared,
        timeout: call.timeout,
        limits: pipeline.limits,
        boundaries: pipeline.boundaries,
    };
    let mut state = JobState::new(prepared);
    let outcome = run(&remote, pipeline.audio_files, call, target, &mut state).await;
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
    audio_files: &dyn AuthorizedAudioFiles,
    call: &PipelineCall<'_>,
    target: PipelineTarget<'_, '_>,
    state: &mut JobState,
) -> Result<(), PipelineError>
where
    T: RemoteTransport,
    S: Sleeper,
{
    let PipelineTarget {
        mut audio_file,
        response,
        logger,
    } = target;
    select_reference(remote, audio_files, audio_file.as_deref_mut(), state).await?;
    let body = build_create_body(
        call.model,
        &remote.prepared.params,
        &remote.prepared.options,
        state.selected_file.as_deref(),
    );
    log_pre_call(&mut *logger, &remote.prepared.api_base, &body);
    let created = exchange::create(remote, &body).await?;
    state.transcription = Some(created.id.clone());
    let meta = poll_until_completed(remote, &created.id).await?;
    let transcript = exchange::fetch(remote, &created.id).await?;
    let payload = SonioxTranscriptPayload {
        transcription: meta.raw,
        transcript,
    };
    let format = remote.prepared.options.response_format.as_deref();
    let updated = build_response_from_payload(&payload, Some(response.clone()), format)?;
    *response = updated;
    log_post_call(
        &mut *logger,
        audio_file.as_deref(),
        &original_response(&payload),
        &body,
    );
    hidden::apply(response, call.model, meta.audio_duration_ms.as_ref())?;
    Ok(())
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

async fn select_reference<T, S>(
    remote: &SonioxRemote<'_, T, S>,
    audio_files: &dyn AuthorizedAudioFiles,
    audio_file: Option<&mut AudioFile<'_>>,
    state: &mut JobState,
) -> Result<(), PipelineError>
where
    T: RemoteTransport,
    S: Sleeper,
{
    let audio_url = remote.prepared.options.audio_url.as_deref();
    let has_url = audio_url.is_some_and(|url| !url.is_empty());
    if state.selected_file.is_some() || has_url {
        return Ok(());
    }
    let Some(file) = audio_file else {
        return Err(missing_input());
    };
    let processed = process_audio_file(audio::reborrow(file), audio_files)?;
    let uploaded = upload_file(remote, &processed).await?;
    state.uploaded_file = Some(uploaded.clone());
    state.selected_file = Some(uploaded);
    Ok(())
}

fn missing_input() -> PipelineError {
    PipelineError::Provider(SonioxException {
        message: String::from(MISSING_INPUT),
        status_code: MISSING_INPUT_STATUS,
        headers: None,
    })
}

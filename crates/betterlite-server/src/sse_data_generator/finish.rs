use std::io;
use std::mem::take;
use std::sync::Arc;

use bytes::Bytes;

use super::cleanup::finish;
use super::failure::failure_frame;
use super::stream::Drive;
use super::types::{FailureKind, Flag, StreamFailure};
use crate::stream_chunks::DONE_FRAME;
use crate::stream_chunks::frames::ends_with_frame_delimiter;

pub(super) fn end<Key>(drive: &mut Drive<Key>) -> Vec<Result<Bytes, io::Error>>
where
    Key: Send + Sync + 'static,
{
    drive.finished = Flag(true);
    if let Some(outcome) = drive.session.outcome.as_mut() {
        outcome.stream_completed = true;
    }
    if !drive.session.setup.wrapped.0 {
        drive
            .session
            .hooks
            .fire_deferred_stream_logging(drive.session.request_data.as_ref());
    }
    let mut frames = Vec::new();
    if !drive.raw_buffer.is_empty() {
        let mut frame = take(&mut drive.raw_buffer);
        if !ends_with_frame_delimiter(&frame) {
            frame.push_str("\n\n");
        }
        frames.push(Ok(Bytes::from(frame)));
    }
    if let Some(message) = drive.error_message.take() {
        frames.push(Ok(Bytes::from(message)));
    }
    if !drive.session.setup.skip_done.0 {
        frames.push(Ok(Bytes::from_static(DONE_FRAME.as_bytes())));
    }
    frames
}

pub(super) async fn handle_failure<Key>(
    drive: &mut Drive<Key>,
    failure: StreamFailure,
) -> Vec<Result<Bytes, io::Error>>
where
    Key: Send + Sync + 'static,
{
    tracing::error!(
        "litellm.proxy.proxy_server.async_data_generator(): Exception occurred - {}",
        failure.message
    );
    drive.finished = Flag(true);
    if let Err(hook_error) = call_failure_hook(drive, &failure).await {
        finish(&mut drive.session, false);
        return vec![Err(io::Error::other(hook_error))];
    }
    if drive.session.error_state.is_some() {
        return state_failure(drive, &failure);
    }
    if failure.kind == FailureKind::Http {
        finish(&mut drive.session, false);
        return vec![Err(io::Error::other(failure))];
    }
    if let Some(outcome) = drive.session.outcome.as_mut() {
        outcome.stream_completed = true;
    }
    let frame = failure_frame(&failure);
    vec![Ok(Bytes::from(frame))]
}

async fn call_failure_hook<Key>(
    drive: &mut Drive<Key>,
    failure: &StreamFailure,
) -> Result<(), StreamFailure>
where
    Key: Send + Sync + 'static,
{
    let Some(key) = drive.session.outcome.as_ref().map(|outcome| &outcome.key) else {
        return Ok(());
    };
    let hooks = Arc::clone(&drive.session.hooks);
    hooks
        .failure_hook(key, failure, drive.session.request_data.as_ref())
        .await
}

fn state_failure<Key>(
    drive: &mut Drive<Key>,
    failure: &StreamFailure,
) -> Vec<Result<Bytes, io::Error>>
where
    Key: Send + Sync + 'static,
{
    if let Some(outcome) = drive.session.outcome.as_mut() {
        outcome.stream_completed = true;
    }
    let frame = drive
        .session
        .error_state
        .as_mut()
        .and_then(|state| state.format_failure(failure));
    let mut frames = frame
        .into_iter()
        .map(|frame| Ok(Bytes::from(frame)))
        .collect::<Vec<_>>();
    if !drive.session.setup.skip_done.0 {
        frames.push(Ok(Bytes::from_static(DONE_FRAME.as_bytes())));
    }
    frames
}

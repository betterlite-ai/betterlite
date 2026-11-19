use super::{Prefetched, RunOutcome, Step, StepOutcome, ViewOutcome};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::wrapper_stream::NORMALIZED_ID;
use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_core::api::messages_chat_stream::sse::SyncFrames;
use betterlite_core::api::messages_chat_stream::view::AnthropicSseStream;
use betterlite_models::decisions;

pub(super) async fn async_steps(
    mut view: AnthropicSseStream<Prefetched, UuidV4>,
    steps: &[Step],
) -> Result<RunOutcome, SdkHttpError> {
    let mut outcomes = Vec::with_capacity(steps.len());
    for step in steps {
        let (frames, end) = async_step(&mut view, *step).await?;
        outcomes.push(StepOutcome {
            frames,
            end,
            view: Some(view_outcome(&view)),
        });
    }
    Ok(RunOutcome {
        is_async: true,
        steps: outcomes,
    })
}

async fn async_step(
    view: &mut AnthropicSseStream<Prefetched, UuidV4>,
    step: Step,
) -> Result<(Vec<String>, bool), SdkHttpError> {
    match step {
        Step::Close => {
            view.aclose();
            Ok((Vec::new(), false))
        }
        Step::Next => match view.next_frame().await {
            Some(frame) => {
                let frame = normalized_frame(&frame, &view.stream.state.message_id)?;
                Ok((vec![frame], false))
            }
            None => Ok((Vec::new(), true)),
        },
        Step::Drain => drain_async(view).await,
    }
}

async fn drain_async(
    view: &mut AnthropicSseStream<Prefetched, UuidV4>,
) -> Result<(Vec<String>, bool), SdkHttpError> {
    let mut frames = Vec::new();
    while let Some(frame) = view.next_frame().await {
        frames.push(normalized_frame(&frame, &view.stream.state.message_id)?);
    }
    Ok((frames, true))
}

pub(super) fn sync_steps(
    mut frames: SyncFrames<Prefetched, UuidV4>,
    steps: &[Step],
) -> Result<RunOutcome, SdkHttpError> {
    let mut outcomes = Vec::with_capacity(steps.len());
    for step in steps {
        let (collected, end) = sync_step(&mut frames, *step)?;
        outcomes.push(StepOutcome {
            frames: collected,
            end,
            view: None,
        });
    }
    Ok(RunOutcome {
        is_async: false,
        steps: outcomes,
    })
}

fn sync_step(
    frames: &mut SyncFrames<Prefetched, UuidV4>,
    step: Step,
) -> Result<(Vec<String>, bool), SdkHttpError> {
    match step {
        Step::Next => match frames.next() {
            Some(Ok(frame)) => {
                let frame = normalized_frame(&frame, &frames.driver.state.message_id)?;
                Ok((vec![frame], false))
            }
            Some(Err(error)) => Err(SdkHttpError::Invalid(error.to_string())),
            None => Ok((Vec::new(), true)),
        },
        Step::Drain => drain_sync(frames),
        Step::Close => Err(SdkHttpError::Invalid(
            "a sync run cannot be closed".to_owned(),
        )),
    }
}

fn drain_sync(
    frames: &mut SyncFrames<Prefetched, UuidV4>,
) -> Result<(Vec<String>, bool), SdkHttpError> {
    let mut collected = Vec::new();
    while let Some(item) = frames.next() {
        let frame = item.map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        collected.push(normalized_frame(&frame, &frames.driver.state.message_id)?);
    }
    Ok((collected, true))
}

// The message id is a fresh uuid4 on each side, so both replace it before comparing
fn normalized_frame(frame: &[u8], message_id: &str) -> Result<String, SdkHttpError> {
    Ok(String::from_utf8(frame.to_vec())
        .map_err(SdkHttpError::FrameUtf8)?
        .replace(message_id, NORMALIZED_ID))
}

fn view_outcome(view: &AnthropicSseStream<Prefetched, UuidV4>) -> ViewOutcome {
    ViewOutcome {
        model: view.model().to_owned(),
        messages: view.messages().map(<[decisions::Json]>::to_vec),
        chunks: view
            .chunks()
            .map(|chunks| chunks.iter().map(|chunk| chunk.id.clone()).collect()),
    }
}

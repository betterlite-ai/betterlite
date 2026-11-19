use super::{Prefetched, RunOutcome, Step, StepOutcome, ViewOutcome};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::wrapper_stream::NORMALIZED_ID;
use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_core::api::messages_chat_stream::sse::SyncFrames;
use betterlite_core::api::messages_chat_stream::view::AnthropicSseStream;
use betterlite_models::decisions;

// One step loop drives both factory modes; each variant hands over its next frame and its view.
pub(super) enum Pump {
    Async(Box<AnthropicSseStream<Prefetched, UuidV4>>),
    Sync(Box<SyncFrames<Prefetched, UuidV4>>),
}

impl Pump {
    async fn pull(&mut self) -> Option<Result<Vec<u8>, SdkHttpError>> {
        match self {
            Self::Async(view) => view.next_frame().await.map(Ok),
            Self::Sync(frames) => frames
                .next()
                .map(|item| item.map_err(|error| SdkHttpError::Invalid(error.to_string()))),
        }
    }

    fn close(&mut self) -> Result<(), SdkHttpError> {
        let Self::Async(view) = self else {
            return Err(SdkHttpError::Invalid(
                "a sync run cannot be closed".to_owned(),
            ));
        };
        view.aclose();
        Ok(())
    }

    fn message_id(&self) -> String {
        match self {
            Self::Async(view) => view.stream.state.message_id.clone(),
            Self::Sync(frames) => frames.driver.state.message_id.clone(),
        }
    }

    fn view(&self) -> Option<ViewOutcome> {
        match self {
            Self::Async(view) => Some(view_outcome(view)),
            Self::Sync(_) => None,
        }
    }
}

pub(super) async fn run_steps(mut pump: Pump, steps: &[Step]) -> Result<RunOutcome, SdkHttpError> {
    let mut outcomes = Vec::with_capacity(steps.len());
    for step in steps {
        let (frames, end) = one_step(&mut pump, *step).await?;
        outcomes.push(StepOutcome {
            frames,
            end,
            view: pump.view(),
        });
    }
    Ok(RunOutcome {
        // The wrapper's sync generator has no close, so a case never asks the sync mode to close.
        is_async: matches!(pump, Pump::Async(_)),
        steps: outcomes,
    })
}

async fn one_step(pump: &mut Pump, step: Step) -> Result<(Vec<String>, bool), SdkHttpError> {
    match step {
        Step::Close => {
            pump.close()?;
            Ok((Vec::new(), false))
        }
        Step::Next => {
            let pulled = pump.pull().await;
            match pulled {
                Some(frame) => Ok((vec![normalized_frame(&frame?, &pump.message_id())?], false)),
                None => Ok((Vec::new(), true)),
            }
        }
        Step::Drain => Ok((drain(pump).await?, true)),
    }
}

async fn drain(pump: &mut Pump) -> Result<Vec<String>, SdkHttpError> {
    let mut frames = Vec::new();
    loop {
        let Some(item) = pump.pull().await else {
            return Ok(frames);
        };
        frames.push(normalized_frame(&item?, &pump.message_id())?);
    }
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

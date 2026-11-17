use std::collections::VecDeque;

use async_trait::async_trait;
use axum::body::Bytes;

#[async_trait]
pub(crate) trait FrameSource<Error: Send + 'static>: Send {
    async fn fill(&mut self, pending: &mut VecDeque<Bytes>) -> Result<bool, Error>;
}

pub(crate) struct Pump<Source> {
    source: Source,
    pending: VecDeque<Bytes>,
    finished: bool,
}

impl<Source> Pump<Source> {
    pub(crate) const fn new(source: Source) -> Self {
        Self {
            source,
            pending: VecDeque::new(),
            finished: false,
        }
    }
}

pub(crate) async fn next_frame<Source, Error>(
    mut state: Pump<Source>,
) -> Option<(Result<Bytes, Error>, Pump<Source>)>
where
    Source: FrameSource<Error> + Send,
    Error: Send + 'static,
{
    loop {
        if let Some(frame) = state.pending.pop_front() {
            return Some((Ok(frame), state));
        }
        if state.finished {
            return None;
        }
        match state.source.fill(&mut state.pending).await {
            Ok(false) => {}
            Ok(true) => state.finished = true,
            Err(error) => {
                state.finished = true;
                return Some((Err(error), state));
            }
        }
    }
}

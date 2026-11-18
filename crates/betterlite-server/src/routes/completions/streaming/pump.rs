use std::collections::VecDeque;

use async_trait::async_trait;

#[async_trait]
pub(crate) trait FrameSource<Error: Send + 'static>: Send {
    type Item: Send + 'static;

    async fn fill(&mut self, pending: &mut VecDeque<Self::Item>) -> Result<bool, Error>;
}

pub(crate) struct Pump<Source, Item> {
    source: Source,
    pending: VecDeque<Item>,
    finished: bool,
}

impl<Source, Item> Pump<Source, Item> {
    pub(crate) const fn new(source: Source) -> Self {
        Self {
            source,
            pending: VecDeque::new(),
            finished: false,
        }
    }
}

pub(crate) async fn next_frame<Source, Error>(
    mut state: Pump<Source, Source::Item>,
) -> Option<(Result<Source::Item, Error>, Pump<Source, Source::Item>)>
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

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;

use super::{
    CompatibleLineChunk, Mode, Outcome, ReceiveError, StreamError, decode_legacy_compatible_line,
    receive_failure,
};

pub type LineItem<E> = Result<String, ReceiveError<E>>;
pub type ChunkItem<E> = Result<CompatibleLineChunk, StreamError<E>>;

#[derive(Debug)]
pub struct SyncLines<S> {
    pub source: S,
    pub finished: bool,
}

#[derive(Debug)]
pub struct AsyncLines<S> {
    pub source: S,
    pub finished: bool,
}

impl<S> From<S> for SyncLines<S> {
    fn from(source: S) -> Self {
        Self {
            source,
            finished: false,
        }
    }
}

impl<S> From<S> for AsyncLines<S> {
    fn from(source: S) -> Self {
        Self {
            source,
            finished: false,
        }
    }
}

fn chunk_item<E>(item: Option<LineItem<E>>, mode: Mode) -> Option<ChunkItem<E>> {
    match item? {
        Ok(line) => match decode_legacy_compatible_line(&line, mode) {
            Outcome::Chunk(chunk) => Some(Ok(chunk)),
            Outcome::End => None,
        },
        Err(error) => Some(Err(receive_failure(error, mode))),
    }
}

impl<S, E> Iterator for SyncLines<S>
where
    S: Iterator<Item = LineItem<E>>,
{
    type Item = ChunkItem<E>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let produced = chunk_item(self.source.next(), Mode::Sync);
        self.finished = !matches!(produced, Some(Ok(_)));
        produced
    }
}

impl<S, E> Stream for AsyncLines<S>
where
    S: Stream<Item = LineItem<E>> + Unpin,
{
    type Item = ChunkItem<E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let lines = self.get_mut();
        if lines.finished {
            return Poll::Ready(None);
        }
        let polled = Pin::new(&mut lines.source)
            .poll_next(cx)
            .map(|item| chunk_item(item, Mode::Async));
        if let Poll::Ready(produced) = &polled {
            lines.finished = !matches!(produced, Some(Ok(_)));
        }
        polled
    }
}

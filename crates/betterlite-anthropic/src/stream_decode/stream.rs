use std::error::Error as StdError;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures_util::Stream;

use crate::usage::ReasoningTokenCounter;

use super::error::DecodeError;
use super::step::{Progress, step};
use super::{ChunkItem, ChunkStream, DecodedChunk};

type StreamItem<SourceFailure, CounterFailure> =
    Result<DecodedChunk, DecodeError<SourceFailure, CounterFailure>>;

impl<Source, SourceFailure, Counter> Iterator for ChunkStream<Source, Counter>
where
    Source: Iterator<Item = ChunkItem<SourceFailure>>,
    SourceFailure: StdError + 'static,
    Counter: ReasoningTokenCounter,
{
    type Item = StreamItem<SourceFailure, Counter::Failure>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let received = if self.source_ended {
                None
            } else {
                self.source.next()
            };
            self.source_ended = received.is_none();
            match step(&mut self.converter, received) {
                Ok(Progress::Produced(chunk)) => return Some(Ok(chunk)),
                Ok(Progress::Skipped) => {}
                Ok(Progress::Ended) => return None,
                Err(error) => return Some(Err(error)),
            }
        }
    }
}

impl<Source, SourceFailure, Counter> Stream for ChunkStream<Source, Counter>
where
    Source: Stream<Item = ChunkItem<SourceFailure>> + Unpin,
    SourceFailure: StdError + 'static,
    Counter: ReasoningTokenCounter + Unpin,
{
    type Item = StreamItem<SourceFailure, Counter::Failure>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let received = if this.source_ended {
                None
            } else {
                ready!(Pin::new(&mut this.source).poll_next(cx))
            };
            this.source_ended = received.is_none();
            match step(&mut this.converter, received) {
                Ok(Progress::Produced(chunk)) => return Poll::Ready(Some(Ok(chunk))),
                Ok(Progress::Skipped) => {}
                Ok(Progress::Ended) => return Poll::Ready(None),
                Err(error) => return Poll::Ready(Some(Err(error))),
            }
        }
    }
}

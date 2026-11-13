use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use betterlite_vertex::legacy_prediction::streamer::TextStreamer;
use futures_util::Stream;
use serde_json::{Value, json};

use super::input::{Input, Step};

const STOP_SYNC: &str = "StopIteration";
const STOP_ASYNC: &str = "StopAsyncIteration";

pub(crate) fn run(input: &Input) -> Value {
    let mut words_streamer = TextStreamer::new(&input.text);
    let mut async_streamer = TextStreamer::new(&input.text);
    let mut trace_streamer = TextStreamer::new(&input.text);
    json!({
        "words": sync_words(&mut words_streamer),
        "asyncWords": async_words(&mut async_streamer),
        "trace": trace(&mut trace_streamer, &input.steps),
    })
}

fn sync_words(streamer: &mut TextStreamer) -> Vec<String> {
    let mut words = Vec::new();
    while let Some(word) = Iterator::next(streamer) {
        words.push(word);
    }
    words
}

fn async_words(streamer: &mut TextStreamer) -> Vec<String> {
    let mut words = Vec::new();
    while let Some(word) = next_async(streamer) {
        words.push(word);
    }
    words
}

fn trace(streamer: &mut TextStreamer, steps: &[Step]) -> Vec<String> {
    let mut trace = Vec::with_capacity(steps.len());
    for step in steps {
        trace.push(match step {
            Step::Sync => Iterator::next(streamer).unwrap_or_else(|| STOP_SYNC.to_owned()),
            Step::Async => next_async(streamer).unwrap_or_else(|| STOP_ASYNC.to_owned()),
        });
    }
    trace
}

fn next_async(streamer: &mut TextStreamer) -> Option<String> {
    let mut context = Context::from_waker(Waker::noop());
    match Pin::new(streamer).poll_next(&mut context) {
        Poll::Ready(item) => item,
        // a TextStreamer has every word ready, so its stream never polls pending
        Poll::Pending => None,
    }
}

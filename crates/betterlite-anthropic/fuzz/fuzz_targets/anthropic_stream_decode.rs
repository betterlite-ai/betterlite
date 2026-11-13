#![no_main]

use std::convert::Infallible;

use betterlite_anthropic::stream_decode::decode_anthropic_event;
use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_state::state::{StreamOptions, StreamState, UuidV4};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use libfuzzer_sys::fuzz_target;

struct NoFailures;

impl ReasoningTokenCounter for NoFailures {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

fuzz_target!(|data: &[u8]| {
    let state = StreamState::new(StreamOptions::default(), &UuidV4);
    let mut converter = StreamEventConverter {
        state,
        counter: NoFailures,
        json_could_close: false,
    };
    let _outcome = decode_anthropic_event(&mut converter, data);
});

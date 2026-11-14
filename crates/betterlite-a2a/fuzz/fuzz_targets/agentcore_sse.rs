#![no_main]

use std::convert::Infallible;

use betterlite_a2a::agentcore_transform::sse::parse_sse_events;
use futures_util::{StreamExt as _, stream};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let lines = stream::iter(
        text.lines()
            .map(|line| Ok::<_, Infallible>(line.to_owned())),
    );
    futures_executor::block_on(async {
        let mut events = std::pin::pin!(parse_sse_events(lines));
        while let Some(event) = events.next().await {
            drop(event);
        }
    });
});

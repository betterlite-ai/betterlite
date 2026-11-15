use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::pin::pin;
use std::str::from_utf8;

use futures_util::{StreamExt as _, stream};
use rstest::rstest;

use crate::agentcore_transform::parse_sse_events;

#[rstest]
#[tokio::test]
async fn replays_the_agentcore_sse_corpus(#[files("fuzz/corpus/agentcore_sse/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = from_utf8(&bytes) else {
        return;
    };
    let lines = stream::iter(
        text.lines()
            .map(|line| Ok::<_, Infallible>(line.to_owned())),
    );

    let mut events = pin!(parse_sse_events(lines));
    while let Some(event) = events.next().await {
        event.expect("the event is not an error");
    }
}

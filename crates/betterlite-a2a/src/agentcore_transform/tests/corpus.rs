use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;

use futures_util::{StreamExt as _, stream};
use rstest::rstest;

use crate::agentcore_transform::parse_sse_events;

#[rstest]
#[tokio::test]
async fn replays_the_agentcore_sse_corpus(#[files("fuzz/corpus/agentcore_sse/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    let lines = stream::iter(
        text.lines()
            .map(|line| Ok::<_, Infallible>(line.to_owned())),
    );

    let mut events = std::pin::pin!(parse_sse_events(lines));
    while let Some(event) = events.next().await {
        assert!(event.is_ok());
    }
}

#![cfg(test)]

mod decoder;
mod passthrough;
mod stream;
mod support;
mod translation;

use std::fs;
use std::path::Path;

use crate::api::api_boundaries::ApiLimits;

#[test]
fn replays_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_chat_event_line");
    let mut iterator = support::stream();
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let _replayed = iterator.parse_line(&bytes, &ApiLimits::default());
        })
        .count();
    assert!(replayed >= 6);
}

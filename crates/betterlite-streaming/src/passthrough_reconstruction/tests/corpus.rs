use std::fs;
use std::path::Path;

use crate::passthrough_reconstruction::collapse::collapse_pure_text_chunks;
use crate::passthrough_reconstruction::events::{RawSseChunk, split_sse_chunk_into_events};

const MERGED: &str = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello world\"}}";

#[test]
fn replays_the_anthropic_relay_reconstruction_corpus() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_relay_reconstruction");
    let entries: Vec<_> = fs::read_dir(root).unwrap().collect();
    assert!(!entries.is_empty());
    entries
        .into_iter()
        .try_for_each(|entry| {
            let path = entry.unwrap().path();
            let fragment = fs::read_to_string(&path)?;
            let chunk = RawSseChunk::Text(fragment);
            let events = split_sse_chunk_into_events(&chunk).unwrap();
            assert!(!events.is_empty(), "{path:?}");
            let collapsed = collapse_pure_text_chunks(&[chunk]).unwrap();
            match path.file_name().unwrap().to_str().unwrap() {
                "pure-text" => {
                    assert_eq!(events.len(), 8);
                    let merged = collapsed.expect("a pure text stream collapses");
                    assert_eq!(merged.len(), 6);
                    assert_eq!(merged[2], MERGED);
                }
                "tool-block" | "thinking-delta" | "bare-events" | "done-marker"
                | "malformed-frame" => assert!(collapsed.is_none(), "{path:?}"),
                name => panic!("missing outcome for corpus input {name}"),
            }
            Ok::<(), std::io::Error>(())
        })
        .unwrap();
}

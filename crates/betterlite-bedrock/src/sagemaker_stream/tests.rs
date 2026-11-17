#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use betterlite_http::error::HttpError;

use crate::sagemaker_stream::events::EventItem;
use crate::sagemaker_stream::{ErrorPolicy, SagemakerChunk, SagemakerStreamDecoder};

const MODEL: &str = "sagemaker-model";

// the fuzz target reads the first byte as the chunk size and feeds the rest as one byte stream
fn decode(bytes: &[u8], messages: bool, policy: ErrorPolicy) -> Vec<EventItem> {
    let size = usize::from(bytes.first().copied().unwrap_or(0)).max(1);
    let decoder = SagemakerStreamDecoder::new(MODEL, messages, policy);
    let chunks: Vec<Result<Vec<u8>, HttpError>> = bytes
        .get(1..)
        .unwrap_or_default()
        .chunks(size)
        .map(|chunk| Ok(chunk.to_vec()))
        .collect();
    decoder.events(chunks.into_iter()).collect()
}

fn is_token_chunk(item: &EventItem) -> bool {
    matches!(item, Ok(Some(SagemakerChunk::Token(_))))
}

fn is_chat_chunk(item: &EventItem) -> bool {
    matches!(item, Ok(Some(SagemakerChunk::Chat(_))))
}

fn check_seed(name: &str, bytes: &[u8]) {
    let propagate = decode(bytes, false, ErrorPolicy::Propagate);
    let skip = decode(bytes, false, ErrorPolicy::Skip);

    if name.starts_with("invalid-") {
        assert!(propagate.iter().any(Result::is_err), "{name} fails");
        assert!(skip.iter().all(Result::is_ok), "{name} is skipped");
        return;
    }
    if name.starts_with("truncated-") {
        assert!(propagate.is_empty(), "{name} carries no whole event");
        return;
    }
    if name == "leftover-buffer" {
        assert!(matches!(propagate.as_slice(), [Ok(None)]), "{name}");
        assert!(matches!(skip.as_slice(), [Ok(None)]), "{name}");
        return;
    }
    if name == "messages-chunk" {
        let messages = decode(bytes, true, ErrorPolicy::Propagate);
        assert!(
            messages.iter().any(is_chat_chunk),
            "{name} projects a chat chunk"
        );
    }
    assert!(
        propagate.iter().any(is_token_chunk),
        "{name} projects a token chunk"
    );
}

#[test]
fn replays_the_sagemaker_stream_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/sagemaker_stream");
    let mut seen = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            check_seed(&name, &bytes);
            seen = seen.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seen >= 10, "expected committed seeds, found {seen}");
}

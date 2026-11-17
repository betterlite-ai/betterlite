use std::fs;
use std::path::Path;

use crate::response::decode_anthropic_reply;
use crate::response::error::DecodeError;
use crate::response::wire::AnthropicReply;

fn named(path: &Path, names: &[&str]) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| names.contains(&name))
}

#[test]
fn replays_the_anthropic_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_response");
    let sources = fs::read_dir(corpus)
        .expect("the response corpus should exist")
        .map(|entry| {
            let path = entry.expect("a corpus entry").path();
            let bytes = fs::read(path.clone()).expect("a corpus seed should be readable");
            (path, bytes)
        })
        .collect::<Vec<_>>();
    assert!(sources.iter().any(|(path, bytes)| {
        named(path, &["valid.json", "boundary.json"])
            && matches!(
                decode_anthropic_reply(bytes),
                Ok(AnthropicReply::Success(_))
            )
    }));
    assert!(sources.iter().any(|(path, bytes)| {
        named(path, &["boundary.json"])
            && matches!(
                decode_anthropic_reply(bytes),
                Ok(AnthropicReply::Success(response)) if response.content.is_empty()
            )
    }));
    assert!(sources.iter().any(|(path, bytes)| {
        named(path, &["malformed.json", "truncated.json"])
            && matches!(
                decode_anthropic_reply(bytes),
                Err(DecodeError::MalformedJson { .. })
            )
    }));
    assert!(sources.iter().any(|(path, bytes)| {
        named(path, &["not-an-object.json"])
            && matches!(decode_anthropic_reply(bytes), Err(DecodeError::NotAnObject))
    }));
}

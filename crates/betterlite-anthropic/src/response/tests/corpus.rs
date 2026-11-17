use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::response::decode_anthropic_reply;
use crate::response::error::DecodeError;
use crate::response::wire::AnthropicReply;

#[test]
fn replays_the_anthropic_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_response");
    let decoded = fs::read_dir(corpus)
        .expect("the response corpus should exist")
        .map(|entry| {
            let path = entry.expect("a corpus entry").path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a corpus seed name is text")
                .to_owned();
            let bytes = fs::read(path).expect("a corpus seed should be readable");
            (name, decode_anthropic_reply(&bytes))
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(decoded.len(), 5, "the corpus holds five seeds");
    assert!(
        matches!(
            decoded.get("valid.json"),
            Some(Ok(AnthropicReply::Success(response))) if !response.content.is_empty()
        ),
        "valid.json should decode to a content list"
    );
    assert!(
        matches!(
            decoded.get("boundary.json"),
            Some(Ok(AnthropicReply::Success(response))) if response.content.is_empty()
        ),
        "boundary.json should decode to an empty content list"
    );
    for name in ["malformed.json", "truncated.json"] {
        assert!(
            matches!(
                decoded.get(name),
                Some(Err(DecodeError::MalformedJson { .. }))
            ),
            "{name} should classify as malformed JSON"
        );
    }
    assert!(
        matches!(
            decoded.get("not-an-object.json"),
            Some(Err(DecodeError::NotAnObject))
        ),
        "not-an-object.json should classify as not an object"
    );
}

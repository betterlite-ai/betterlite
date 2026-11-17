use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::response::decode_anthropic_reply;
use crate::response::error::DecodeError;
use crate::response::wire::AnthropicReply;

type Decoded = BTreeMap<String, Result<AnthropicReply, DecodeError>>;

fn decode_corpus() -> Decoded {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_response");
    fs::read_dir(corpus)
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
        .collect()
}

fn seed<'decoded>(
    decoded: &'decoded Decoded,
    name: &str,
) -> &'decoded Result<AnthropicReply, DecodeError> {
    decoded
        .get(name)
        .unwrap_or_else(|| panic!("the corpus holds {name}"))
}

#[test]
fn replays_the_anthropic_response_corpus() {
    let decoded = decode_corpus();
    assert!(decoded.len() >= 5, "the corpus holds at least five seeds");
    assert!(
        matches!(
            seed(&decoded, "valid.json"),
            Ok(AnthropicReply::Success(response)) if !response.content.is_empty()
        ),
        "valid.json should decode to a content list"
    );
    assert!(
        matches!(
            seed(&decoded, "boundary.json"),
            Ok(AnthropicReply::Success(response)) if response.content.is_empty()
        ),
        "boundary.json should decode to an empty content list"
    );
    assert!(
        matches!(
            seed(&decoded, "malformed.json"),
            Err(DecodeError::MalformedJson { .. })
        ),
        "malformed.json should classify as malformed JSON"
    );
    assert!(
        matches!(
            seed(&decoded, "truncated.json"),
            Err(DecodeError::MalformedJson { .. })
        ),
        "truncated.json should classify as malformed JSON"
    );
    assert!(
        matches!(
            seed(&decoded, "not-an-object.json"),
            Err(DecodeError::NotAnObject)
        ),
        "not-an-object.json should classify as not an object"
    );
}

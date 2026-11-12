use std::fs;
use std::path::Path;

use betterlite_models::decisions::Json;

use crate::count_call::reply::decode_bedrock_count_reply;

#[test]
fn replays_the_bedrock_other_count_call_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_other_count_call");
    let sources = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read(path).unwrap())
        })
        .collect::<Vec<_>>();
    assert!(sources.len() >= 8);
    assert!(sources.iter().all(|(name, bytes)| {
        let decodes = !name.starts_with("malformed") && !name.starts_with("truncated");
        assert_eq!(decode_bedrock_count_reply(bytes).is_ok(), decodes, "{name}");
        true
    }));
    let (_, valid) = sources
        .iter()
        .find(|(name, _)| name == "valid_input_tokens.json")
        .unwrap();
    let decoded = decode_bedrock_count_reply(valid).unwrap();
    assert_eq!(
        decoded.get("inputTokens"),
        Some(&Json::Number(serde_json::Number::from(42_u8)))
    );
}

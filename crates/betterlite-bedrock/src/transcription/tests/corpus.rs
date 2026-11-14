use std::fs;
use std::path::Path;

use crate::transcription::reply::decode_transcription_reply;

#[test]
fn replays_the_bedrock_transcription_reply_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_transcription_reply");
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
    for (name, bytes) in &sources {
        let decodes = !name.starts_with("malformed") && !name.starts_with("truncated");
        assert_eq!(decode_transcription_reply(bytes).is_ok(), decodes, "{name}");
    }
    let texts = [
        ("valid_two_blocks.json", "hello world"),
        ("valid_empty_content.json", ""),
        ("boundary_unicode_text.json", "h\u{e9}llo \u{1f3a4}"),
    ];
    for (file, expected) in texts {
        let (_, bytes) = sources.iter().find(|(name, _)| name == file).unwrap();
        assert_eq!(decode_transcription_reply(bytes).unwrap().text, expected);
    }
}

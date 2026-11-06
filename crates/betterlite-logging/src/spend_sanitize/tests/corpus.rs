use std::fs;
use std::path::Path;
use std::str;

use crate::spend_sanitize::prompt_leaks::redact_spend_error_text;

const MAX_BYTES: usize = 64 * 1024;

#[test]
fn replays_the_spend_error_text_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/spend_error_text");
    let names = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
            let bytes = fs::read(&path).unwrap();
            let text = str::from_utf8(&bytes[..bytes.len().min(MAX_BYTES)]).unwrap();
            let redacted = redact_spend_error_text(text);
            assert_eq!(
                redacted.contains("REDACTED_BY_LITELM"),
                name.ends_with("-redacts"),
                "{name}"
            );
            assert_eq!(redact_spend_error_text(&redacted), redacted, "{name}");
            name
        })
        .collect::<Vec<_>>();
    assert!(names.len() >= 10);
}

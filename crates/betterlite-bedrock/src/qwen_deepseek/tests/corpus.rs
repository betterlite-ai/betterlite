#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::qwen_deepseek::QwenProjection;
use crate::qwen_deepseek::error::QwenReplyError;
use crate::qwen_deepseek::tests::support::{QwenRun, fixture, qwen_bytes};

#[test]
fn replays_the_bedrock_qwen_deepseek_corpus() {
    let seeds = replay_bedrock_qwen_deepseek_corpus();
    assert!(seeds >= 10, "the corpus holds only {seeds} seeds");
}

fn replay_bedrock_qwen_deepseek_corpus() -> usize {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_qwen_deepseek");
    let mut seeds = 0_usize;
    for entry in fs::read_dir(corpus).expect("the corpus directory exists") {
        let path = entry.expect("the corpus entry reads").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is UTF-8")
            .to_owned();
        let bytes = fs::read(&path).expect("the seed reads");
        let case = fixture();
        for projection in [QwenProjection::Qwen2, QwenProjection::Qwen3] {
            let run = qwen_bytes(&case, projection, &bytes);
            assert_category(&name, projection, &run);
        }
        seeds = seeds.saturating_add(1);
    }
    seeds
}

fn assert_category(name: &str, projection: QwenProjection, run: &QwenRun) {
    let expected_ok = name.starts_with("valid-") || name.starts_with("boundary-");
    match (&run.reply, expected_ok) {
        (Ok(_reply), true) => {}
        (Err(error), true) => panic!("{name} ({projection:?}) failed as {error}"),
        (Ok(_reply), false) => panic!("{name} ({projection:?}) decoded as a reply"),
        (Err(error), false) => assert!(
            category_matches(name, error),
            "{name} ({projection:?}) failed as {error}"
        ),
    }
}

fn category_matches(name: &str, error: &QwenReplyError) -> bool {
    if name.starts_with("malformed-json-") || name.starts_with("truncated-") {
        return matches!(error, QwenReplyError::Decode(_));
    }
    if name.starts_with("malformed-not-object-") {
        return matches!(error, QwenReplyError::NotObject);
    }
    if name.starts_with("malformed-not-text-") {
        return matches!(error, QwenReplyError::NotText { .. });
    }
    if name.starts_with("malformed-usage-") {
        return matches!(
            error,
            QwenReplyError::UsageNotObject | QwenReplyError::UsageCount { .. }
        );
    }
    panic!("{name} names no expected category");
}

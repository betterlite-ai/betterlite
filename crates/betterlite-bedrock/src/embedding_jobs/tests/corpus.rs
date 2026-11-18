use std::fs;
use std::path::Path;

use super::super::{EmbeddingJobsError, decode_async_embedding_status};

fn category(bytes: &[u8]) -> &'static str {
    match decode_async_embedding_status(bytes) {
        Ok(_) => "accepted",
        Err(EmbeddingJobsError::Decode { .. }) => "rejected",
        Err(_) => "unexpected",
    }
}

#[test]
fn replays_the_bedrock_other_embedding_jobs_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_other_embedding_jobs");
    let mut count = 0_usize;
    for entry in fs::read_dir(corpus).expect("the corpus exists") {
        let path = entry.expect("the seed entry reads").path();
        let bytes = fs::read(&path).expect("the seed reads");
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("the seed name is UTF-8")
            .to_owned();
        let accepted = name.starts_with("valid") || name.starts_with("boundary-valid");
        let expected = if accepted { "accepted" } else { "rejected" };
        assert_eq!(category(&bytes), expected, "seed {name}");
        count = count.saturating_add(1);
    }
    assert!(count > 0, "the corpus holds seeds");
}

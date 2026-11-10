use std::fs;
use std::path::PathBuf;

use crate::spend_bodies::classifier::decode_classifier_snapshot;

#[rstest::rstest]
fn replays_the_classifier_snapshot_corpus(
    #[files("fuzz/corpus/classifier_snapshot/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _snapshot = decode_classifier_snapshot(&bytes);
}

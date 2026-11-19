use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::affinity_claims::pin::{PinError, StoredPin};

#[rstest]
fn replays_the_affinity_corpus(#[files("fuzz/corpus/affinity/affinity-claims-*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let outcome = StoredPin::parse(&bytes);
    if name.contains("invalid") {
        assert_eq!(outcome.unwrap_err(), PinError::NotText);
    } else {
        assert_eq!(
            outcome.unwrap().pinned_model_id().as_deref(),
            Some("deployment-1")
        );
    }
}

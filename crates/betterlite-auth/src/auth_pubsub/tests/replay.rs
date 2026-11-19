use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::auth_pubsub::decode::decode_invalidation;
use crate::auth_pubsub::message::read;

#[rstest]
fn replays_the_auth_pubsub_corpus(#[files("fuzz/corpus/auth_pubsub/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus seed");
    let name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .expect("a seed name")
        .to_owned();
    if name.contains("invalid") {
        assert!(
            decode_invalidation(&bytes).is_err(),
            "{name} must not decode"
        );
    } else {
        let typed = decode_invalidation(&bytes).expect("a valid seed");
        assert!(
            read(&typed).is_some(),
            "{name} must decode to an invalidation"
        );
    }
}

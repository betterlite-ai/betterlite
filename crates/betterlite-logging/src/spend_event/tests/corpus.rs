use std::fs;
use std::path::PathBuf;

use crate::spend_event::decode::decode_spend_event;
use crate::spend_event::restore::callback_args;

// A corpus file named ...-valid decodes and restores, and one named ...-rejected must not decode
#[rstest::rstest]
fn replays_the_spend_event_corpus(#[files("fuzz/corpus/spend_event/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let decoded = decode_spend_event(&bytes);
    if name.ends_with("-valid") {
        let event = decoded.unwrap();
        let encoded = serde_json::to_vec(&event).unwrap();
        assert_eq!(decode_spend_event(&encoded).unwrap(), event, "{name}");
        callback_args(event).unwrap();
    } else if name.ends_with("-rejected") {
        assert!(decoded.is_err(), "{name}");
    } else {
        drop(decoded);
    }
}

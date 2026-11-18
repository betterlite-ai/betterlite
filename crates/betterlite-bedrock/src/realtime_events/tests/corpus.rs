use std::fs;
use std::path::PathBuf;

use crate::realtime_events::decode::{
    DecodedEvent, decode_realtime_client_preface, decode_realtime_native_event,
};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_realtime_events")
}

// The seed name ends with the outcome the decoder must reach, so a parser that starts accepting
// a malformed frame fails here instead of fuzzing on
fn outcome(bytes: &[u8]) -> &'static str {
    match decode_realtime_native_event(bytes) {
        Ok(DecodedEvent::Event(_)) => "event",
        Ok(DecodedEvent::InvalidJson { .. }) => "warning",
        Err(_) => "rejected",
    }
}

#[test]
fn replays_the_bedrock_services_realtime_events_corpus() {
    let mut replayed = 0_usize;
    for entry in fs::read_dir(corpus_dir()).expect("the corpus directory") {
        let path = entry.expect("a corpus entry").path();
        let bytes = fs::read(&path).expect("a corpus file");
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("a seed name");
        let category = name.rsplit('-').next().expect("a seed category");
        assert_eq!(outcome(&bytes), category, "{name}");
        drop(decode_realtime_client_preface(&bytes));
        replayed += 1;
    }
    assert!(replayed >= 8, "the corpus keeps its seeds");
}

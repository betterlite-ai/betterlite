use std::fs;
use std::path::Path;

use super::{async_stream, drain, metadata, sync_events, sync_stream};
use crate::google_genai::provider::GenAiProvider;

const CORPUS_SEEDS: usize = 7;

#[tokio::test]
async fn replays_the_genai_sse_corpus() {
    let seeds = corpus_seeds();
    assert_eq!(seeds.len(), CORPUS_SEEDS);
    replay_all(seeds).await;
}

fn corpus_seeds() -> Vec<(String, Vec<u8>)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/genai_sse");
    let mut seeds: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in fs::read_dir(corpus).expect("the corpus folder") {
        let entry = entry.expect("a corpus entry");
        let bytes = fs::read(entry.path()).expect("a seed file");
        let name = entry
            .file_name()
            .to_str()
            .expect("the seed name is UTF-8")
            .to_owned();
        seeds.push((name, bytes));
    }
    seeds.sort();
    seeds
}

async fn replay_all(seeds: Vec<(String, Vec<u8>)>) {
    for (name, bytes) in seeds {
        let whole = replay(vec![bytes.clone()]);
        let byte_wise: Vec<Vec<u8>> = bytes.chunks(1).map(<[u8]>::to_vec).collect();
        assert_eq!(
            replay(byte_wise),
            whole,
            "{name} parses the same byte by byte"
        );
        let mut stream = async_stream(vec![bytes.clone()], metadata(GenAiProvider::Gemini), None);
        assert_eq!(
            drain(&mut stream).await,
            whole,
            "{name} parses the same asynchronously"
        );
        assert!(
            whole.iter().all(|event| event.ends_with(b"\n\n")),
            "{name} ends every event with a blank line"
        );
        check_corpus_expectation(&name, &bytes, &whole);
    }
}

fn replay(chunks: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut stream = sync_stream(chunks);
    sync_events(&mut stream)
}

fn check_corpus_expectation(name: &str, bytes: &[u8], events: &[Vec<u8>]) {
    match name {
        "blank-line-delimiter" | "unterminated-final-event" => assert_eq!(
            events,
            [
                b"data: {\"a\":1}\n\n".to_vec(),
                b"data: {\"b\":2}\n\n".to_vec()
            ]
        ),
        "crlf-lines" => assert_eq!(events, [b"data: {\"a\":1}\n\n".to_vec()]),
        "empty" => assert!(events.is_empty()),
        "invalid-utf8" => check_replacement_decoding(events),
        "large-inline-media" | "split-utf8" => assert_eq!(events, [bytes.to_vec()]),
        other => panic!("the corpus seed {other} has no expectation"),
    }
}

fn check_replacement_decoding(events: &[Vec<u8>]) {
    let [event] = events else {
        panic!("the invalid UTF-8 seed yields one event");
    };
    assert!(event.starts_with(b"data: "));
    assert!(event.ends_with(b"\n\n"));
    assert!(
        event
            .windows(3)
            .any(|window| window == b"\xEF\xBF\xBD".as_slice())
    );
}

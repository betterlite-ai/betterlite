#![no_main]

use betterlite_core::google_genai::streaming::GoogleGenAiStream;
use libfuzzer_sys::fuzz_target;

const LINE_LIMIT: usize = 0x0010_0000;

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Event(Vec<u8>),
    Failed,
}

fuzz_target!(|data: &[u8]| {
    let whole = events(vec![data.to_vec()]);
    let byte_wise = events(data.chunks(1).map(<[u8]>::to_vec).collect());
    assert_eq!(whole, byte_wise);
    for step in &whole {
        let Step::Event(event) = step else {
            continue;
        };
        assert!(event.ends_with(b"\n\n"));
    }
});

// the steps handed out must not depend on where the reads split the input
fn events(chunks: Vec<Vec<u8>>) -> Vec<Step> {
    let stream = GoogleGenAiStream::new(chunks.into_iter().map(Ok), LINE_LIMIT);
    stream
        .map(|item| item.map_or(Step::Failed, Step::Event))
        .collect()
}

#![no_main]

use betterlite_core::google_genai::metadata::GenAiStreamMetadata;
use betterlite_core::google_genai::provider::GenAiProvider;
use betterlite_core::google_genai::streaming::{GenAiStreamError, GoogleGenAiStream};
use betterlite_models::decisions::HiddenParams;
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
    let stream = GoogleGenAiStream::new(chunks.into_iter().map(Ok), metadata(), LINE_LIMIT);
    stream
        .map(|item| match item {
            Ok(event) => Step::Event(event),
            Err(_) => Step::Failed,
        })
        .collect()
}

fn metadata() -> GenAiStreamMetadata {
    GenAiStreamMetadata {
        model: String::new(),
        provider: GenAiProvider::Gemini,
        request_body: HiddenParams::new(),
        hidden_params: HiddenParams::new(),
    }
}

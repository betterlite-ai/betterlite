#![no_main]

use betterlite_core::google_genai::metadata::GenAiStreamMetadata;
use betterlite_core::google_genai::provider::GenAiProvider;
use betterlite_core::google_genai::streaming::GoogleGenAiStream;
use betterlite_models::decisions::HiddenParams;
use libfuzzer_sys::fuzz_target;

const LINE_LIMIT: usize = 0x0010_0000;

fuzz_target!(|data: &[u8]| {
    let whole = events(vec![data.to_vec()]);
    let byte_wise = events(data.chunks(1).map(<[u8]>::to_vec).collect());
    assert_eq!(whole, byte_wise);
    for event in &whole {
        let Ok(event) = event else {
            continue;
        };
        assert!(event.ends_with(b"\n\n"));
    }
});

// the events handed out must not depend on where the reads split the input
fn events(chunks: Vec<Vec<u8>>) -> Vec<Result<Vec<u8>, String>> {
    let stream = GoogleGenAiStream::new(chunks.into_iter().map(Ok), metadata(), LINE_LIMIT);
    stream
        .map(|item| item.map_err(ToString::to_string))
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

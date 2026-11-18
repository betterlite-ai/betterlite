#![no_main]
use betterlite_bedrock::sagemaker_stream::{ErrorPolicy, SagemakerStreamDecoder};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let size = usize::from(bytes.first().copied().unwrap_or(0)).max(1);
    let data = bytes.get(1..).unwrap_or_default();
    for policy in [ErrorPolicy::Propagate, ErrorPolicy::Skip] {
        for messages in [false, true] {
            let decoder = SagemakerStreamDecoder::new("sagemaker-model", messages, policy);
            let chunks = data
                .chunks(size)
                .map(|chunk| Ok(chunk.to_vec()))
                .collect::<Vec<_>>();
            drop(decoder.events(chunks.into_iter()).collect::<Vec<_>>());
        }
    }
});

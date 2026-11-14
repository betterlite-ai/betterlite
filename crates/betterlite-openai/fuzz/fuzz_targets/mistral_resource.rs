#![no_main]
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::mistral_batches::records::MistralBatchJob;
use betterlite_openai::mistral_files::decode::decode_mistral_resource;
use betterlite_openai::mistral_files::records::{MistralFile, MistralFileDeleted, MistralFileList};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    };
    let Some((tag, body)) = bytes.split_first() else {
        return;
    };
    let _decoded = match tag {
        1 => decode_mistral_resource::<MistralFile>(body, &limits).map(drop),
        2 => decode_mistral_resource::<MistralFileList>(body, &limits).map(drop),
        3 => decode_mistral_resource::<MistralFileDeleted>(body, &limits).map(drop),
        _ => decode_mistral_resource::<MistralBatchJob>(body, &limits).map(drop),
    };
});

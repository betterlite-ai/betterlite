#![no_main]
use betterlite_openai::batches::decode::decode_batch_object;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _batch = decode_batch_object(bytes);
});

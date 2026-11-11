#![no_main]
use betterlite_bedrock::batch_create::reply::decode_created_batch;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _outcome = decode_created_batch(data);
});

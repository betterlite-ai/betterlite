#![no_main]
use betterlite_bedrock::batch_status::decode::decode_batch_status;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _outcome = decode_batch_status(data, 200, 1_700_000_000);
});

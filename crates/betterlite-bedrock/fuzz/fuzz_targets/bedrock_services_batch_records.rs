#![no_main]

use betterlite_bedrock::batch_records::record::decode_batch_input_line;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_batch_input_line(bytes);
});

#![no_main]
use betterlite_bedrock::s3_access::decode::decode_managed_file_id;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|value: &str| {
    let _outcome = decode_managed_file_id(value);
});

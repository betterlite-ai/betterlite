#![no_main]
use betterlite_bedrock::s3_signing::naming::decode_upload_record;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _outcome = decode_upload_record(data);
});

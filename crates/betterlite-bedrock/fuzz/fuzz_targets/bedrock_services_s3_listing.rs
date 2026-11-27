#![no_main]

use betterlite_bedrock::s3_listing::{DecodeLimits, decode_s3_listing};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = decode_s3_listing(data, &DecodeLimits::DEFAULT);
});

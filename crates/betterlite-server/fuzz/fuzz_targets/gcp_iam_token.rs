#![no_main]
use betterlite_server::redis_cloud_auth::gcp::decode_iam_token;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _token = decode_iam_token(body);
});

#![no_main]
use betterlite_server::release_info::records::decode_fuzzed_latest_release;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _release = decode_fuzzed_latest_release(data);
});

#![no_main]

use betterlite_server::sso_return_targets::decode::decode_sso_return_target;
use betterlite_server::sso_return_targets::return_path::is_same_origin_return_path;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some(target) = decode_sso_return_target(data) else {
        return;
    };
    assert!(!is_same_origin_return_path(&target) || target.starts_with('/'));
});

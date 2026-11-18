#![no_main]
use betterlite_config::spend_attribution::decode_spend_proxy_selection;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _settings = decode_spend_proxy_selection(bytes);
});

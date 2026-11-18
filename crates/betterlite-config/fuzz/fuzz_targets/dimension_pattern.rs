#![no_main]
use betterlite_config::sections::custom_dimensions::pattern::pattern_work;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(pattern) = str::from_utf8(data) {
        drop(pattern_work(pattern));
    }
});

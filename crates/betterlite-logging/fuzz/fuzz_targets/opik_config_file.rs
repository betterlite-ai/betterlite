#![no_main]

use betterlite_logging::opik::config_file::read_config_section;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        let _outcome = read_config_section(text);
    }
});

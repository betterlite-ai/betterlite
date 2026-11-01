#![no_main]

use betterlite_logging::opik::config_file::read_config_section;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = std::str::from_utf8(bytes) {
        let _ = read_config_section(text);
    }
});

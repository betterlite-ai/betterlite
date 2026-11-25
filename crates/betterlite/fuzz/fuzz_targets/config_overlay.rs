#![no_main]

use std::{path::Path, str::from_utf8};

use betterlite::db_config_overlay::source::parse_config_file;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    drop(parse_config_file(Path::new("proxy.yaml"), text));
});

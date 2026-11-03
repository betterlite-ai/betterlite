#![no_main]

use betterlite_oracle::case::helper::validate_helper;
use std::path::Path;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let source = String::from_utf8_lossy(bytes);
    let _result = serde_saphyr::from_str(&source)
        .map_err(|error| error.to_string())
        .and_then(|case| {
            validate_helper(Path::new("<helper>"), &case).map_err(|error| error.to_string())
        });
});

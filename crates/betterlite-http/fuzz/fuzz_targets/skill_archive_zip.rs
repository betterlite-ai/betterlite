#![no_main]

use betterlite_http::skill_archive::build_skill_archive;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _outcome = build_skill_archive(input);
});

#![no_main]

use betterlite_router::tag_matchers::document::parse_tag_match;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = parse_tag_match(data);
});

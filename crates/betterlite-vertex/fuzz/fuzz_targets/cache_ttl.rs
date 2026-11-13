#![no_main]

use betterlite_vertex::cache_prefix::ttl::parse_cache_ttl;
use jiff::Timestamp;
use libfuzzer_sys::fuzz_target;

const NOW_SECOND: i64 = 1_794_441_600;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(now) = Timestamp::from_second(NOW_SECOND) {
        let _normalized = parse_cache_ttl(bytes, now);
    }
});

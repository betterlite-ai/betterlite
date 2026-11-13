#![no_main]

use betterlite_cache::envelope::decode::{decode_cached, parse_cached_form};
use libfuzzer_sys::fuzz_target;

const MAX_AGE_SECONDS: Option<i64> = Some(60);
const FIXTURE_NOW: f64 = 1_700_000_000.0;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = parse_cached_form(bytes)
        .and_then(|cached| decode_cached(Some(&cached), MAX_AGE_SECONDS, FIXTURE_NOW));
});

#![no_main]

use betterlite_db::spend_attribution::tags::decode_enqueue_tags;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_enqueue_tags(bytes);
});

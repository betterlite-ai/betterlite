#![no_main]
use betterlite_logging::spend_bodies::classifier::decode_classifier_snapshot;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _snapshot = decode_classifier_snapshot(bytes);
});

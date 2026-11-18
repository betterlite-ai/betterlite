#![no_main]

use betterlite_router::tier_artifact_predictor::artifact::parse_artifact;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = parse_artifact(data);
});

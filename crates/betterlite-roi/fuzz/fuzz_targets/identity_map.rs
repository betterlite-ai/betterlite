#![no_main]

use betterlite_roi::summary::identity_map::ROIIdentityMapUpdate;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = serde_json::from_slice::<ROIIdentityMapUpdate>(bytes);
});

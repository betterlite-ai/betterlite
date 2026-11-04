#![no_main]

use betterlite_oracle::websocket_oracle::{
    OracleLimits,
    decode::{FilesystemFixtures, decode_ws_case},
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_ws_case(
        bytes,
        &OracleLimits::default(),
        &FilesystemFixtures::manifest_default(),
    );
});

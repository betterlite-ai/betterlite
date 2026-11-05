#![no_main]

use std::path::PathBuf;

use betterlite_oracle::websocket_oracle::{
    OracleLimits,
    decode::{FilesystemFixtures, decode_ws_case},
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let fixtures = FilesystemFixtures {
        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/websocket-oracle"),
    };
    let _decoded = decode_ws_case(bytes, &OracleLimits::default(), &fixtures);
});

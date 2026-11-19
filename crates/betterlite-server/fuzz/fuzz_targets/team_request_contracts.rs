#![no_main]

use betterlite_server::team_request_contracts::decode_team_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_team_request(bytes);
});

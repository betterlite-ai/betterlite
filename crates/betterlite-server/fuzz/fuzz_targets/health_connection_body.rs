#![no_main]

use betterlite_server::health_access_and_connections::connection_test::ConnectionTestRequest;
use betterlite_server::workflow_runs::router::decode_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = decode_body::<ConnectionTestRequest>(data);
});

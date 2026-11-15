#![no_main]

use betterlite_logging::azure_storage_logging::token::parse_token_response;
use betterlite_logging::azure_storage_logging::transport::StorageResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let response = StorageResponse {
        status: 200,
        body: body.to_vec(),
    };
    let _token = parse_token_response(&response);
});

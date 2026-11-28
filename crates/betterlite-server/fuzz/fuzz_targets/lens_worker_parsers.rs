#![no_main]

use axum::http::HeaderValue;
use betterlite_server::lens_worker_routes::{claim, content, credential};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let authorization = HeaderValue::from_bytes(bytes).ok();
    drop(claim::protocol_version(bytes));
    drop(content::execution_id(bytes));
    drop(credential::bearer_token(authorization.as_ref()));
});

#![no_main]

use axum::http::HeaderValue;
use betterlite_server::lens_worker_routes::{claim, content, credential};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let authorization = HeaderValue::from_bytes(bytes).ok();
    if let Ok(version) = claim::protocol_version(bytes) {
        // an accepted version is an integer, so its decimal text parses back to it
        assert_eq!(
            claim::protocol_version(version.to_string().as_bytes()),
            Ok(version)
        );
    }
    drop(content::execution_id(bytes));
    drop(credential::bearer_token(authorization.as_ref()));
});

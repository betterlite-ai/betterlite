#![no_main]

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_server::mcp::outbound_credentials::authz_code_refresher::grant_body::read_grant_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    if let Ok(JsonValue::Object(grant)) = serde_json::from_slice::<JsonValue>(body) {
        drop(read_grant_body(&grant));
    }
});

#![no_main]

use betterlite_server::mcp::oauth_protected::upstream_fetch::parse_metadata_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _outcome = parse_metadata_body(body);
});

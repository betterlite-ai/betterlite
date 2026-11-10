#![no_main]

use betterlite_mcp::oauth_discovery::document::decode_metadata_document;
use betterlite_mcp::oauth_discovery::www_authenticate::parse_www_authenticate;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _challenge = parse_www_authenticate(bytes);
    let _document = decode_metadata_document(bytes);
});

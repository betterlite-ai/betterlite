#![no_main]

use betterlite_guardrails::mcp_jwt_signer::idp_documents::document::decode_idp_document;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _document = decode_idp_document(bytes);
});

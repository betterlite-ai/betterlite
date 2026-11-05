#![no_main]

use betterlite_guardrails::{prisma_airs_extraction, prisma_airs_masking};
use libfuzzer_sys::fuzz_target;
use serde_json::value::RawValue;
use std::str::from_utf8;

fuzz_target!(|bytes: &[u8]| {
    let _messages = serde_json::from_slice::<Vec<prisma_airs_extraction::Message>>(bytes);
    let _response = serde_json::from_slice::<prisma_airs_extraction::Response>(bytes);
    let _mask = serde_json::from_slice::<prisma_airs_masking::ScanResult>(bytes);
    let _response_mask = serde_json::from_slice::<prisma_airs_masking::Response>(bytes);
    if let Ok(mut request) = prisma_airs_masking::mcp::parse_mcp_request(bytes)
        && let Ok(text) = from_utf8(bytes)
    {
        let _result =
            prisma_airs_masking::mcp::mask_mcp_arguments(&mut request, RawValue::NULL, text);
    }
});

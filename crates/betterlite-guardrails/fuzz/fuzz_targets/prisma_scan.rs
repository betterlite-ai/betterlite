#![no_main]

use betterlite_guardrails::panw_prisma_airs::tool_scan::ToolCall;
use betterlite_guardrails::panw_prisma_airs::verdict::ScanVerdict;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _verdict = ScanVerdict::from_body(bytes);
    let _calls = serde_json::from_slice::<Vec<ToolCall>>(bytes);
});

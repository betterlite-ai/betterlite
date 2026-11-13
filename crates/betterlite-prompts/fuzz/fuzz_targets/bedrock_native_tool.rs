#![no_main]

use betterlite_prompts::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use betterlite_prompts::bedrock_tools::parse::tools_from_json_bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let capabilities = BedrockToolCapabilities {
        cache_points: CachePointSupport::ExtendedTtl,
        supports_strict_tools: true,
    };
    drop(tools_from_json_bytes(bytes, capabilities));
});

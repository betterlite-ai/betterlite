#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_server::api::claude_plugin_read::decode_claude_plugin_read;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _decoded = decode_claude_plugin_read(body, &ApiLimits::default());
});

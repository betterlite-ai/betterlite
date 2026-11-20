#![no_main]

use betterlite_server::plugin_routes::claim::verify_plugin_session_claim;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|token: &str| {
    let _claim = verify_plugin_session_claim("litellm-platform-plugin", token);
});

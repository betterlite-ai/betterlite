#![no_main]

use betterlite_openai::cloudflare_decisions::CloudflareDecisionsConfig;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let config = CloudflareDecisionsConfig::new(None);
    let _unwrapped = config.unwrap_response(bytes);
});

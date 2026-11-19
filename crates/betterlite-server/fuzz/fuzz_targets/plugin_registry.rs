#![no_main]

use betterlite_server::plugin_routes::config::PluginRegistry;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let Ok(settings) = serde_json::from_str(text) else {
        return;
    };
    let registry = PluginRegistry::default();
    let _replaced = registry.replace_from_settings(&settings);
});

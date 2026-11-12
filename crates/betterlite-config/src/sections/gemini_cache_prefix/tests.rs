#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::gemini_cache_prefix::Settings;

fn load(source: &str) -> Settings {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("the config loads")
    .sections
    .gemini_cache_prefix
}

#[test]
fn loads_disable_transform() {
    let settings =
        load("litellm_settings:\n  disable_anthropic_gemini_context_caching_transform: true\n");
    assert!(settings.disable_context_caching);
}

#[test]
fn disable_transform_defaults_to_false() {
    assert!(!load("").disable_context_caching);
    assert!(!load("litellm_settings:\n  drop_params: true\n").disable_context_caching);
    assert!(
        !load("litellm_settings:\n  disable_anthropic_gemini_context_caching_transform: false\n")
            .disable_context_caching
    );
}

#![cfg(test)]

use crate::loader::parse_view;

use super::RawConfig;

#[test]
fn raw_config_debug_hides_secrets() {
    let source = "model_list:\n  - model_name: gpt\n    litellm_params:\n      api_key: marker-api-key\ngeneral_settings:\n  master_key: marker-master-key\n";
    let (raw, _ignored) = parse_view::<RawConfig>(source).expect("config parses");
    let debug = format!("{raw:?}");
    assert!(!debug.contains("marker-api-key"));
    assert!(!debug.contains("marker-master-key"));
}

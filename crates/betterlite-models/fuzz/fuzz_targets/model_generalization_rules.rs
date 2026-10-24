#![no_main]

use betterlite_models::catalog_generalizations::{CatalogGeneralizations, decode_model_generalization_rules};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(rules) = decode_model_generalization_rules(bytes) {
        let (generalizations, _warnings) = CatalogGeneralizations::install(rules);
        let _provider = generalizations.match_routing("claude-x-1");
        let _capabilities = generalizations.match_capabilities("claude-x-1");
        let _fill = generalizations.match_fill_missing("claude-x-1", "anthropic");
    }
});

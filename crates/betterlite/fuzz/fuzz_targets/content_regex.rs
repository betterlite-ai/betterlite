#![no_main]

use betterlite::guardrails::pattern_matching::compile;
use betterlite_config::sections::content_filter_records::{
    ContentFilterAction, ContentFilterPattern,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
    if source.len() <= 4096 {
        let _result = compile(&ContentFilterPattern {
            pattern_type: "regex".to_owned(),
            pattern_name: None,
            pattern: Some(source.to_owned()),
            name: None,
            action: ContentFilterAction::Mask,
        });
    }
});

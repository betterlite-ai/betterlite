#![no_main]

use std::collections::BTreeMap;

use betterlite_router::classifier_verdict::reply::parse_class_reply;
use betterlite_router::tier_definitions::model::TierSet;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let tiers = TierSet {
        definitions: None,
        pools: BTreeMap::new(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    };
    let _outcome = parse_class_reply(data, &tiers);
});

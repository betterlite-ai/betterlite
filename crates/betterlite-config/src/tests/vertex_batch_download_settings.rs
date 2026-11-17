#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

fn conversion_disabled(source: &str) -> bool {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    ConfigLoader { lookup }
        .load(source)
        .expect("the loader accepts the vertex batch output setting")
        .sections
        .batch_cost_finalize
        .disable_output_transformation
}

#[test]
fn loads_disable_vertex_batch_output_transformation() {
    assert!(!conversion_disabled(""));
    assert!(!conversion_disabled(
        "litellm_settings:\n  disable_vertex_batch_output_transformation: null\n"
    ));
    assert!(conversion_disabled(
        "litellm_settings:\n  disable_vertex_batch_output_transformation: true\n"
    ));
    assert!(!conversion_disabled(
        "litellm_settings:\n  disable_vertex_batch_output_transformation: false\n"
    ));
}

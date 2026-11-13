#![cfg(test)]

use toml_edit::DocumentMut;

use crate::codex_settings::snapshot::{Snapshot, with_value};

#[test]
fn with_value_leaves_a_non_table_section_alone() {
    let document = "model_providers = \"not a table\"\n"
        .parse::<DocumentMut>()
        .unwrap();
    let provider = Snapshot::parse("[value]\nname = \"gw\"\n").unwrap();
    let updated = with_value(&document, "model_providers.litellm", Some(&provider));
    assert_eq!(updated.to_string(), document.to_string());
    let removed = with_value(&document, "model_providers.litellm", None);
    assert_eq!(removed.to_string(), document.to_string());
}

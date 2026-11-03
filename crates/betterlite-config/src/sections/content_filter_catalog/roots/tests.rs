#![cfg(test)]

use std::path::Path;

use super::{CATEGORIES_DIR, DATA_DIR, DATA_ROOTS, LEGACY_DATA_DIR, POLICY_TEMPLATES_DIR};

#[test]
fn bundled_content_paths_and_root_order_are_exposed() {
    assert_eq!(
        Path::new(DATA_DIR).join("categories"),
        Path::new(CATEGORIES_DIR)
    );
    assert_eq!(
        Path::new(DATA_DIR).join("policy_templates"),
        Path::new(POLICY_TEMPLATES_DIR)
    );
    assert_eq!(DATA_ROOTS, [DATA_DIR, LEGACY_DATA_DIR]);
}

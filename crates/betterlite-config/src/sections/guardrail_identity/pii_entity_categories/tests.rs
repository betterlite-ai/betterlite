#![cfg(test)]

use super::super::enums::{PII_ENTITY_CATEGORIES_MAP, PiiEntityCategory};

#[test]
fn map_has_expected_groups() {
    assert_eq!(PII_ENTITY_CATEGORIES_MAP.len(), 20);
    assert_eq!(PII_ENTITY_CATEGORIES_MAP[0].0, PiiEntityCategory::General);
    assert_eq!(PII_ENTITY_CATEGORIES_MAP[0].1.len(), 11);
    assert_eq!(PII_ENTITY_CATEGORIES_MAP[11].1.len(), 13);
}

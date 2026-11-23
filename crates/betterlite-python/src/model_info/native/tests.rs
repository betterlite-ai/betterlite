#![cfg(test)]

use crate::native_api::bundled_model_cost_json;

use super::NativeCatalog;

#[test]
fn model_info_native_generation_moves_with_each_adopt() {
    let text = bundled_model_cost_json().unwrap();
    let catalog = NativeCatalog::new(&text).unwrap();
    assert_eq!(catalog.generation(), 0);
    catalog.adopt("{}").unwrap();
    catalog.adopt("{}").unwrap();
    assert_eq!(catalog.generation(), 2);
}

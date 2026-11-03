#![cfg(test)]

#[test]
fn load_categories_skips_failures_and_keeps_valid_records() {
    super::tests::load_categories_resolves_custom_category_files();
    super::tests::partial_updates_survive_later_load_failure();
}

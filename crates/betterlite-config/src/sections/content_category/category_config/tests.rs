#![cfg(test)]

#[test]
fn category_config_constructor_covers_defaults_and_normalization() {
    super::tests::category_config_default_action_is_block();
    super::tests::category_config_rejects_invalid_action();
    super::tests::category_config_lowercases_words_and_skips_invalid_regex();
}

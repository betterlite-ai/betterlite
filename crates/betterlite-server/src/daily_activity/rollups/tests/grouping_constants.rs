#![cfg(test)]

use super::levels::{
    GROUP_DATE, GROUP_DATE_API_KEY, GROUP_DATE_ENDPOINT, GROUP_DATE_ENDPOINT_API_KEY,
    GROUP_DATE_MCP, GROUP_DATE_MCP_API_KEY, GROUP_DATE_MODEL, GROUP_DATE_MODEL_API_KEY,
    GROUP_DATE_MODEL_GROUP, GROUP_DATE_MODEL_GROUP_API_KEY, GROUP_DATE_PROVIDER,
    GROUP_DATE_PROVIDER_API_KEY, GROUP_GRAND_TOTAL,
};

#[test]
fn grouping_constants_match_litellm() {
    assert_eq!(GROUP_GRAND_TOTAL, 127);
    assert_eq!(GROUP_DATE, 63);
    assert_eq!(GROUP_DATE_API_KEY, 31);
    assert_eq!(GROUP_DATE_MODEL, 47);
    assert_eq!(GROUP_DATE_MODEL_API_KEY, 15);
    assert_eq!(GROUP_DATE_MODEL_GROUP, 55);
    assert_eq!(GROUP_DATE_MODEL_GROUP_API_KEY, 23);
    assert_eq!(GROUP_DATE_PROVIDER, 59);
    assert_eq!(GROUP_DATE_PROVIDER_API_KEY, 27);
    assert_eq!(GROUP_DATE_MCP, 61);
    assert_eq!(GROUP_DATE_MCP_API_KEY, 29);
    assert_eq!(GROUP_DATE_ENDPOINT, 62);
    assert_eq!(GROUP_DATE_ENDPOINT_API_KEY, 30);
}

#[test]
fn grand_total_rolls_up_every_column() {
    assert_eq!(GROUP_GRAND_TOTAL.count_ones(), 7);
    assert_eq!(GROUP_DATE & 64, 0);
    assert_eq!(GROUP_DATE_MODEL & 16, 0);
    assert_eq!(GROUP_DATE_ENDPOINT_API_KEY & 1, 0);
}

#[test]
fn every_level_keeps_the_date_bit_clear() {
    assert_eq!(GROUP_DATE & 64, 0);
    assert_eq!(GROUP_DATE_API_KEY & 64, 0);
    assert_eq!(GROUP_DATE_MODEL & 64, 0);
    assert_eq!(GROUP_DATE_MODEL_API_KEY & 64, 0);
    assert_eq!(GROUP_DATE_MODEL_GROUP & 64, 0);
    assert_eq!(GROUP_DATE_MODEL_GROUP_API_KEY & 64, 0);
    assert_eq!(GROUP_DATE_PROVIDER & 64, 0);
    assert_eq!(GROUP_DATE_PROVIDER_API_KEY & 64, 0);
    assert_eq!(GROUP_DATE_MCP & 64, 0);
    assert_eq!(GROUP_DATE_MCP_API_KEY & 64, 0);
    assert_eq!(GROUP_DATE_ENDPOINT & 64, 0);
    assert_eq!(GROUP_DATE_ENDPOINT_API_KEY & 64, 0);
}

#[test]
fn keyset_levels_clear_the_api_key_bit() {
    assert_eq!(GROUP_DATE_API_KEY & 32, 0);
    assert_eq!(GROUP_DATE_MODEL_API_KEY & 32, 0);
    assert_eq!(GROUP_DATE_MODEL_GROUP_API_KEY & 32, 0);
    assert_eq!(GROUP_DATE_PROVIDER_API_KEY & 32, 0);
    assert_eq!(GROUP_DATE_MCP_API_KEY & 32, 0);
    assert_eq!(GROUP_DATE_ENDPOINT_API_KEY & 32, 0);
}

#[test]
fn rolled_key_levels_set_the_api_key_bit() {
    assert_eq!(GROUP_DATE & 32, 32);
    assert_eq!(GROUP_DATE_MODEL & 32, 32);
    assert_eq!(GROUP_DATE_MODEL_GROUP & 32, 32);
    assert_eq!(GROUP_DATE_PROVIDER & 32, 32);
    assert_eq!(GROUP_DATE_MCP & 32, 32);
    assert_eq!(GROUP_DATE_ENDPOINT & 32, 32);
}

#[test]
fn each_dimension_level_clears_its_own_bit() {
    assert_eq!(GROUP_DATE_MODEL & 16, 0);
    assert_eq!(GROUP_DATE_MODEL_GROUP & 8, 0);
    assert_eq!(GROUP_DATE_PROVIDER & 4, 0);
    assert_eq!(GROUP_DATE_MCP & 2, 0);
    assert_eq!(GROUP_DATE_ENDPOINT & 1, 0);
}

#[test]
fn each_keyed_dimension_level_clears_its_own_bit() {
    assert_eq!(GROUP_DATE_MODEL_API_KEY & 16, 0);
    assert_eq!(GROUP_DATE_MODEL_GROUP_API_KEY & 8, 0);
    assert_eq!(GROUP_DATE_PROVIDER_API_KEY & 4, 0);
    assert_eq!(GROUP_DATE_MCP_API_KEY & 2, 0);
    assert_eq!(GROUP_DATE_ENDPOINT_API_KEY & 1, 0);
}

#[test]
fn each_dimension_level_rolls_up_the_other_dimensions() {
    assert_eq!(GROUP_DATE_MODEL & 15, 15);
    assert_eq!(GROUP_DATE_MODEL_GROUP & 0b1_0111, 16 + 4 + 2 + 1);
    assert_eq!(GROUP_DATE_PROVIDER & 0b1_1011, 16 + 8 + 2 + 1);
    assert_eq!(GROUP_DATE_MCP & 31, 16 + 8 + 4 + 1);
    assert_eq!(GROUP_DATE_ENDPOINT & 31, 16 + 8 + 4 + 2);
}

#[test]
fn each_keyed_dimension_level_rolls_up_the_other_dimensions() {
    assert_eq!(GROUP_DATE_MODEL_API_KEY, 15);
    assert_eq!(GROUP_DATE_MODEL_GROUP_API_KEY, 23);
    assert_eq!(GROUP_DATE_PROVIDER_API_KEY, 27);
    assert_eq!(GROUP_DATE_MCP_API_KEY, 29);
    assert_eq!(GROUP_DATE_ENDPOINT_API_KEY, 30);
}

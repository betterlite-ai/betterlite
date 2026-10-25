#![cfg(test)]

use rstest::rstest;

use crate::otel::vocabulary::constants::attribute_keys::{
    db, error, gen_ai, json_rpc, litellm, metric,
};

#[rstest]
#[case::deprecated_system(gen_ai::SYSTEM, "gen_ai.system")]
#[case::cache_creation(
    gen_ai::USAGE_CACHE_CREATION_TOKENS,
    "gen_ai.usage.cache_creation.input_tokens"
)]
#[case::rpc_namespace(json_rpc::RESPONSE_STATUS_CODE, "rpc.response.status_code")]
#[case::legacy_error(error::MESSAGE_LEGACY, "error")]
#[case::legacy_db_system(db::SYSTEM_LEGACY, "db.system")]
#[case::cost_prefix(litellm::COST_PREFIX, "litellm.cost.")]
#[case::guardrail_cost(litellm::GUARDRAIL_COST, "litellm.cost.guardrail")]
#[case::token_cost(metric::TOKEN_COST, "gen_ai.usage.cost")]
fn attribute_keys_keep_litellm_spelling(#[case] key: &str, #[case] expected: &str) {
    assert_eq!(key, expected);
}

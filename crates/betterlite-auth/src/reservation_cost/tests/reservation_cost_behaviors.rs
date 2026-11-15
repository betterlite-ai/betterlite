#![cfg(test)]

use std::collections::BTreeMap;

use super::support::{FixedRouter, Fixture, dec, deployment, json};

const FLAT: &str = r#"{"mode":"chat","max_input_tokens":100000,"max_output_tokens":1000,"input_cost_per_token":0.000002,"output_cost_per_token":0.00001}"#;
const OUTPUT_ONLY: &str =
    r#"{"mode":"chat","max_output_tokens":1000,"output_cost_per_token":0.00001}"#;
const TIERED: &str = r#"{"mode":"chat","max_input_tokens":1000000,"max_output_tokens":1000,"tiered_pricing":[{"input_cost_per_token":5e-08,"output_cost_per_token":4e-07,"range":[0,256000]},{"input_cost_per_token":2.5e-07,"output_cost_per_token":2e-06,"range":[256000,1000000]}]}"#;

fn table(entries: &str) -> String {
    format!(
        r#"{{"mode":"chat","max_input_tokens":100000,"max_output_tokens":1000,"tiered_pricing":{entries}}}"#
    )
}

#[tokio::test]
async fn flat_pricing_reserves_input_and_output_tokens() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":100}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.003")));
}

#[tokio::test]
async fn flat_pricing_input_cost_reserves_input_only() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":100}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.input_cost().await.unwrap(), Some(dec("0.002")));
}

#[tokio::test]
async fn provided_counts_skip_the_tokenizer() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":100}"#)
        .model_info("flat-model", FLAT)
        .token_count("flat-model", 7)
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.003")));
    assert_eq!(fixture.input_cost().await.unwrap(), Some(dec("0.002")));
    assert!(fixture.tokens().counted_models().is_empty());
}

#[tokio::test]
async fn one_model_counts_once_across_every_pricing_shape() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([("grouped".to_owned(), json(FLAT))]),
            deployments: BTreeMap::from([(
                "grouped".to_owned(),
                vec![
                    deployment(Some("dep-a"), Some("backend")),
                    deployment(Some("dep-b"), Some("backend")),
                ],
            )]),
            deployment_infos: BTreeMap::from([
                (
                    "dep-a".to_owned(),
                    json(&table(
                        r#"[{"input_cost_per_token":1e-06,"output_cost_per_token":2e-06,"range":[0,5000]}]"#,
                    )),
                ),
                (
                    "dep-b".to_owned(),
                    json(&table(
                        r#"[{"input_cost_per_token":4e-06,"output_cost_per_token":3e-05,"range":[0,5000]}]"#,
                    )),
                ),
            ]),
            ..FixedRouter::default()
        })
        .token_count("grouped", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0043")));
    assert_eq!(
        fixture.tokens().counted_models(),
        vec!["grouped".to_owned()]
    );
}

#[tokio::test]
async fn tiered_pricing_selects_the_tier_for_the_input_count() {
    let fixture = Fixture::new(r#"{"model":"tiered","max_tokens":1000}"#)
        .model_info("tiered", TIERED)
        .provided_count("tiered", 300_000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.077")));
    assert_eq!(fixture.input_cost().await.unwrap(), Some(dec("0.075")));
}

#[tokio::test]
async fn image_generation_sums_both_per_image_fields() {
    let fixture = Fixture::new(r#"{"model":"image-model","n":2}"#).model_info(
        "image-model",
        r#"{"mode":"image_generation","input_cost_per_image":0.0011,"output_cost_per_image":0.134}"#,
    );
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.2702")));
}

#[tokio::test]
async fn image_generation_defaults_to_one_image() {
    let fixture = Fixture::new(r#"{"model":"image-model"}"#).model_info(
        "image-model",
        r#"{"mode":"image_generation","input_cost_per_image":0.0011,"output_cost_per_image":0.134}"#,
    );
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.1351")));
}

#[tokio::test]
async fn image_edit_mode_is_priced_per_image() {
    let fixture = Fixture::new(r#"{"model":"edit-model"}"#).model_info(
        "edit-model",
        r#"{"mode":"image_edit","output_cost_per_image":0.052}"#,
    );
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.052")));
}

#[tokio::test]
async fn chat_mode_with_image_fields_falls_through_to_tokens() {
    let fixture = Fixture::new(r#"{"model":"vision-model","max_tokens":100}"#)
        .model_info(
            "vision-model",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":0.000002,"output_cost_per_token":0.00001,"output_cost_per_image":0.00012}"#,
        )
        .provided_count("vision-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.003")));
}

#[tokio::test]
async fn zero_per_image_cost_falls_through_to_tokens() {
    let fixture = Fixture::new(r#"{"model":"free-image","max_tokens":100}"#)
        .model_info(
            "free-image",
            r#"{"mode":"image_generation","max_output_tokens":1000,"input_cost_per_token":0.000002,"output_cost_per_token":0.00001,"input_cost_per_image":0,"output_cost_per_image":0}"#,
        )
        .provided_count("free-image", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.003")));
}

#[tokio::test]
async fn embedding_route_skips_output_cost() {
    let fixture = Fixture::new(r#"{"model":"embed-model"}"#)
        .route("/v1/embeddings")
        .model_info(
            "embed-model",
            r#"{"mode":"embedding","max_input_tokens":8192,"input_cost_per_token":0.00006}"#,
        )
        .provided_count("embed-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.06")));
}

#[tokio::test]
async fn rerank_route_skips_output_cost() {
    let fixture = Fixture::new(r#"{"model":"rerank-model"}"#)
        .route("/v1/rerank")
        .model_info(
            "rerank-model",
            r#"{"mode":"rerank","max_input_tokens":8192,"input_cost_per_token":0.00006}"#,
        )
        .provided_count("rerank-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.06")));
}

#[tokio::test]
async fn moderation_route_skips_output_cost() {
    let fixture = Fixture::new(r#"{"model":"moderation-model"}"#)
        .route("/v1/moderations")
        .model_info(
            "moderation-model",
            r#"{"mode":"moderation","input_cost_per_token":0.0000005}"#,
        )
        .provided_count("moderation-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0005")));
}

#[tokio::test]
async fn reasoning_rate_reserves_the_higher_rate() {
    let fixture = Fixture::new(r#"{"model":"reason-model","max_tokens":100}"#)
        .model_info(
            "reason-model",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":0.00001,"output_cost_per_token":0.00001,"output_cost_per_reasoning_token":0.00003}"#,
        )
        .provided_count("reason-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.013")));
}

#[tokio::test]
async fn reasoning_rate_alone_is_enough() {
    let fixture = Fixture::new(r#"{"model":"reason-model","max_tokens":100}"#)
        .model_info(
            "reason-model",
            r#"{"mode":"chat","max_output_tokens":1000,"output_cost_per_reasoning_token":0.00003}"#,
        )
        .provided_count("reason-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.003")));
}

#[tokio::test]
async fn n_multiplies_the_output_cost() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":10,"n":3}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0005")));
}

#[tokio::test]
async fn best_of_multiplies_the_output_cost() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":10,"best_of":4}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0006")));
}

#[tokio::test]
async fn the_larger_of_n_and_best_of_wins() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":10,"n":2,"best_of":5}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0007")));
}

#[tokio::test]
async fn zero_and_negative_multipliers_leave_the_default() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":10,"n":0,"best_of":-2}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0003")));
}

#[tokio::test]
async fn max_completion_tokens_wins_over_the_other_fields() {
    let fixture = Fixture::new(
        r#"{"model":"flat-model","max_completion_tokens":10,"max_tokens":200,"max_output_tokens":300}"#,
    )
    .model_info("flat-model", FLAT)
    .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0003")));
}

#[tokio::test]
async fn max_tokens_is_read_when_max_completion_tokens_is_absent() {
    let fixture =
        Fixture::new(r#"{"model":"flat-model","max_tokens":200,"max_output_tokens":300}"#)
            .model_info("flat-model", FLAT)
            .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0022")));
}

#[tokio::test]
async fn max_output_tokens_is_read_last() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_output_tokens":300}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0032")));
}

#[tokio::test]
async fn inference_config_max_tokens_is_the_documented_fallback() {
    let fixture = Fixture::new(r#"{"model":"flat-model","inferenceConfig":{"maxTokens":250}}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0025")));
}

#[tokio::test]
async fn a_non_object_inference_config_is_ignored() {
    let fixture = Fixture::new(r#"{"model":"flat-model","inferenceConfig":"250"}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.01")));
}

#[tokio::test]
async fn an_absent_output_cap_uses_the_model_ceiling() {
    let fixture = Fixture::new(r#"{"model":"flat-model"}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.01")));
}

#[tokio::test]
async fn a_huge_request_is_clamped_to_the_model_ceiling() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":999999999}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.01")));
}

#[tokio::test]
async fn a_huge_request_is_clamped_to_16384_without_a_ceiling() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":999999999}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","output_cost_per_token":0.00001}"#,
        )
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.16384")));
}

#[tokio::test]
async fn a_zero_ceiling_reads_as_absent() {
    let fixture = Fixture::new(r#"{"model":"flat-model"}"#)
        .model_info("flat-model", OUTPUT_ONLY.replace("1000", "0").as_str())
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.16384")));
}

#[tokio::test]
async fn an_unknown_input_rate_skips_the_reservation() {
    let fixture = Fixture::new(r#"{"model":"output-model","max_tokens":10}"#)
        .model_info("output-model", OUTPUT_ONLY)
        .provided_count("output-model", 10);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
    assert_eq!(fixture.input_cost().await.unwrap(), None);
}

#[tokio::test]
async fn an_unknown_output_rate_skips_the_reservation() {
    let fixture = Fixture::new(r#"{"model":"input-model","max_tokens":10}"#)
        .model_info(
            "input-model",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":0.000002}"#,
        )
        .provided_count("input-model", 10);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn zero_tokens_need_no_rate() {
    let fixture = Fixture::new(r#"{"model":"unpriced","max_tokens":0}"#)
        .model_info("unpriced", r#"{"mode":"chat","max_output_tokens":1000}"#)
        .provided_count("unpriced", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0")));
}

#[tokio::test]
async fn a_request_without_a_model_reserves_nothing() {
    let fixture = Fixture::new(r#"{"max_tokens":10}"#);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
    assert_eq!(fixture.input_cost().await.unwrap(), None);
}

#[tokio::test]
async fn multiple_models_reserve_the_maximum() {
    let fixture = Fixture::new(r#"{"model":["cheap-model","dear-model"],"max_tokens":100}"#)
        .model_info(
            "cheap-model",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"input_cost_per_token":0.000001,"output_cost_per_token":0.000002}"#,
        )
        .model_info(
            "dear-model",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"input_cost_per_token":0.000004,"output_cost_per_token":0.00001}"#,
        )
        .provided_count("cheap-model", 1000)
        .provided_count("dear-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.005")));
}

#[tokio::test]
async fn a_model_with_no_known_price_is_ignored() {
    let fixture = Fixture::new(r#"{"model":["dear-model","mystery-model"],"max_tokens":100}"#)
        .model_info(
            "dear-model",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"input_cost_per_token":0.000004,"output_cost_per_token":0.00001}"#,
        )
        .provided_count("dear-model", 1000)
        .provided_count("mystery-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.005")));
}

#[tokio::test]
async fn the_model_input_ceiling_replaces_an_uncountable_request() {
    let fixture =
        Fixture::new(r#"{"model":"flat-model","max_tokens":10}"#).model_info("flat-model", FLAT);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.2001")));
}

#[tokio::test]
async fn provided_counts_replace_the_model_input_ceiling() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":10}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 500);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0011")));
}

#[tokio::test]
async fn input_cost_takes_the_maximum_across_pricing_shapes() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([("grouped".to_owned(), json(FLAT))]),
            deployments: BTreeMap::from([(
                "grouped".to_owned(),
                vec![deployment(Some("dep-a"), Some("backend"))],
            )]),
            deployment_infos: BTreeMap::from([(
                "dep-a".to_owned(),
                json(&table(
                    r#"[{"input_cost_per_token":3e-06,"output_cost_per_token":2e-06,"range":[0,5000]}]"#,
                )),
            )]),
            ..FixedRouter::default()
        })
        .provided_count("grouped", 1000);
    assert_eq!(fixture.input_cost().await.unwrap(), Some(dec("0.003")));
}

#[tokio::test]
async fn the_router_group_info_is_preferred_over_the_catalog() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":100}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([(
                "grouped".to_owned(),
                json(
                    r#"{"mode":"chat","max_input_tokens":100000,"max_output_tokens":1000,"input_cost_per_token":0.000004,"output_cost_per_token":0.000012}"#,
                ),
            )]),
            ..FixedRouter::default()
        })
        .model_info("grouped", FLAT)
        .provided_count("grouped", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0052")));
}

#[tokio::test]
async fn a_router_without_group_info_uses_the_catalog() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":100}"#)
        .router(FixedRouter::default())
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.003")));
}

#[tokio::test]
async fn a_deployment_tiered_table_is_a_candidate_shape() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([("grouped".to_owned(), json(FLAT))]),
            deployments: BTreeMap::from([(
                "grouped".to_owned(),
                vec![deployment(Some("dep-a"), Some("backend"))],
            )]),
            deployment_infos: BTreeMap::from([(
                "dep-a".to_owned(),
                json(&table(
                    r#"[{"input_cost_per_token":1e-06,"output_cost_per_token":2e-06,"range":[0,1000]},{"input_cost_per_token":3e-06,"output_cost_per_token":4e-06,"range":[1000,100000]}]"#,
                )),
            )]),
            ..FixedRouter::default()
        })
        .provided_count("grouped", 5000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.01504")));
}

#[tokio::test]
async fn a_deployment_without_an_id_is_skipped() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([("grouped".to_owned(), json(FLAT))]),
            deployments: BTreeMap::from([(
                "grouped".to_owned(),
                vec![deployment(None, Some("backend"))],
            )]),
            deployment_infos: BTreeMap::from([(
                "dep-a".to_owned(),
                json(&table(
                    r#"[{"input_cost_per_token":9e-06,"range":[0,100000]}]"#,
                )),
            )]),
            ..FixedRouter::default()
        })
        .provided_count("grouped", 5000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0101")));
}

#[tokio::test]
async fn a_deployment_without_a_backend_model_is_skipped() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([("grouped".to_owned(), json(FLAT))]),
            deployments: BTreeMap::from([(
                "grouped".to_owned(),
                vec![deployment(Some("dep-a"), None)],
            )]),
            deployment_infos: BTreeMap::from([(
                "dep-a".to_owned(),
                json(&table(
                    r#"[{"input_cost_per_token":9e-06,"range":[0,100000]}]"#,
                )),
            )]),
            ..FixedRouter::default()
        })
        .provided_count("grouped", 5000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0101")));
}

#[tokio::test]
async fn an_empty_deployment_table_is_not_a_candidate() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([("grouped".to_owned(), json(FLAT))]),
            deployments: BTreeMap::from([(
                "grouped".to_owned(),
                vec![deployment(Some("dep-a"), Some("backend"))],
            )]),
            deployment_infos: BTreeMap::from([(
                "dep-a".to_owned(),
                json(r#"{"mode":"chat","tiered_pricing":[]}"#),
            )]),
            ..FixedRouter::default()
        })
        .provided_count("grouped", 5000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0101")));
}

#[tokio::test]
async fn an_empty_deployment_table_keeps_the_catalog_tiered_price() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            deployments: BTreeMap::from([(
                "grouped".to_owned(),
                vec![deployment(Some("dep-a"), Some("backend"))],
            )]),
            deployment_infos: BTreeMap::from([(
                "dep-a".to_owned(),
                json(r#"{"mode":"chat","tiered_pricing":[]}"#),
            )]),
            ..FixedRouter::default()
        })
        .model_info(
            "grouped",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"input_cost_per_token":0.00001,"output_cost_per_token":0.0001,"tiered_pricing":[{"input_cost_per_token":1e-06,"output_cost_per_token":2e-06,"range":[0,1000]}]}"#,
        )
        .provided_count("grouped", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.00012")));
}

#[tokio::test]
async fn a_non_list_tiered_pricing_table_is_flat() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":100}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","max_input_tokens":100000,"max_output_tokens":1000,"input_cost_per_token":0.000002,"output_cost_per_token":0.00001,"tiered_pricing":"none"}"#,
        )
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.003")));
}

#[tokio::test]
async fn an_explicit_zero_tier_rate_wins_over_the_flat_rate() {
    let fixture = Fixture::new(r#"{"model":"tiered","max_tokens":0}"#)
        .model_info(
            "tiered",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"input_cost_per_token":0.000002,"tiered_pricing":[{"input_cost_per_token":0,"output_cost_per_token":0,"range":[0,1000]}]}"#,
        )
        .provided_count("tiered", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0")));
}

#[tokio::test]
async fn zero_input_tokens_read_the_flat_rate() {
    let fixture = Fixture::new(r#"{"model":"tiered","max_tokens":10}"#)
        .model_info(
            "tiered",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"input_cost_per_token":0.000002,"output_cost_per_token":0.00001,"tiered_pricing":[{"input_cost_per_token":0,"output_cost_per_token":0,"range":[0,1000]}]}"#,
        )
        .provided_count("tiered", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0001")));
}

#[tokio::test]
async fn counted_input_tokens_are_keyed_by_model() {
    let fixture = Fixture::new(r#"{"model":["model-a","model-b"]}"#)
        .token_count("model-a", 10)
        .token_count("model-b", 20);
    let counts = fixture.counted().await.unwrap();
    assert_eq!(counts.len(), 2);
    assert_eq!(counts.get("model-a"), Some(&10));
    assert_eq!(counts.get("model-b"), Some(&20));
    assert_eq!(fixture.tokens().map_calls(), 1);
}

#[tokio::test]
async fn counted_input_tokens_skip_a_model_the_tokenizer_declines() {
    let fixture = Fixture::new(r#"{"model":["model-a","model-b"]}"#).token_count("model-a", 10);
    let counts = fixture.counted().await.unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts.get("model-a"), Some(&10));
}

#[tokio::test]
async fn counted_input_tokens_are_empty_without_a_model() {
    let fixture = Fixture::new(r#"{"max_tokens":10}"#);
    assert!(fixture.counted().await.unwrap().is_empty());
    assert_eq!(fixture.tokens().map_calls(), 0);
}

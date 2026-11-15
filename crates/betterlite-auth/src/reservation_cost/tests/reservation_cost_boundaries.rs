#![cfg(test)]

use std::collections::BTreeMap;

use super::support::{Failing, FixedRouter, Fixture, dec, deployment, json};
use crate::reservation_cost::error::ReservationCostError;

const FLAT: &str = r#"{"mode":"chat","max_input_tokens":100000,"max_output_tokens":1000,"input_cost_per_token":0.000002,"output_cost_per_token":0.00001}"#;
const TIERED: &str = r#"{"mode":"chat","max_input_tokens":1000000,"max_output_tokens":1000,"tiered_pricing":[{"input_cost_per_token":5e-08,"output_cost_per_token":4e-07,"range":[0,256000]},{"input_cost_per_token":2.5e-07,"output_cost_per_token":2e-06,"range":[256000,1000000]}]}"#;

#[tokio::test]
async fn a_group_metadata_failure_returns_no_estimate() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":10}"#)
        .router(FixedRouter {
            failing: Some(Failing::Group),
            ..FixedRouter::default()
        })
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn a_deployment_list_failure_returns_no_estimate() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([("grouped".to_owned(), json(FLAT))]),
            failing: Some(Failing::Deployments),
            ..FixedRouter::default()
        })
        .provided_count("grouped", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn a_deployment_info_failure_returns_no_estimate() {
    let fixture = Fixture::new(r#"{"model":"grouped","max_tokens":10}"#)
        .router(FixedRouter {
            groups: BTreeMap::from([("grouped".to_owned(), json(FLAT))]),
            deployments: BTreeMap::from([(
                "grouped".to_owned(),
                vec![deployment(Some("dep-a"), Some("backend"))],
            )]),
            failing: Some(Failing::DeploymentInfo),
            ..FixedRouter::default()
        })
        .provided_count("grouped", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn a_catalog_failure_returns_no_estimate() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":10}"#)
        .model_info("flat-model", FLAT)
        .model_info_failure()
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn a_malformed_tiered_table_is_an_error() {
    let fixture = Fixture::new(r#"{"model":"tiered","max_tokens":0}"#)
        .model_info(
            "tiered",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"tiered_pricing":[{"range":"bad"}]}"#,
        )
        .provided_count("tiered", 10);
    assert!(matches!(
        fixture.max_cost().await,
        Err(ReservationCostError::Table(_))
    ));
}

#[tokio::test]
async fn a_malformed_tier_range_is_an_error() {
    let fixture = Fixture::new(r#"{"model":"tiered","max_tokens":0}"#)
        .model_info(
            "tiered",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"tiered_pricing":[{"input_cost_per_token":1e-06,"range":null}]}"#,
        )
        .provided_count("tiered", 10);
    assert!(matches!(
        fixture.max_cost().await,
        Err(ReservationCostError::Tier(_))
    ));
}

#[tokio::test]
async fn an_overflowing_amount_is_an_error() {
    let fixture = Fixture::new(r#"{"model":"huge","max_tokens":0}"#)
        .model_info(
            "huge",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":10000000000}"#,
        )
        .provided_count(
            "huge",
            usize::try_from(i64::MAX).expect("a 64-bit usize holds i64::MAX"),
        );
    assert!(matches!(
        fixture.max_cost().await,
        Err(ReservationCostError::Overflow)
    ));
}

#[tokio::test]
async fn a_negative_requested_cap_is_kept() {
    let fixture = Fixture::new(r#"{"model":"output-model","max_tokens":-5}"#)
        .model_info(
            "output-model",
            r#"{"mode":"chat","max_output_tokens":1000,"output_cost_per_token":0.00001}"#,
        )
        .provided_count("output-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("-0.00005")));
}

#[tokio::test]
async fn zero_images_reserve_one() {
    let fixture = Fixture::new(r#"{"model":"image-model","n":0}"#).model_info(
        "image-model",
        r#"{"mode":"image_generation","input_cost_per_image":0.0011,"output_cost_per_image":0.134}"#,
    );
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.1351")));
}

#[tokio::test]
async fn a_tier_boundary_stays_in_the_lower_tier() {
    let fixture = Fixture::new(r#"{"model":"tiered","max_tokens":0}"#)
        .model_info("tiered", TIERED)
        .provided_count("tiered", 256_000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0128")));
}

#[tokio::test]
async fn a_token_past_the_tier_boundary_reads_the_next_tier() {
    let fixture = Fixture::new(r#"{"model":"tiered","max_tokens":0}"#)
        .model_info("tiered", TIERED)
        .provided_count("tiered", 256_001);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.06400025")));
}

#[tokio::test]
async fn the_embedding_route_match_is_case_sensitive() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":100}"#)
        .route("/v1/Embeddings")
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.003")));
}

#[tokio::test]
async fn a_float_token_limit_is_truncated() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":10.9}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","max_input_tokens":100000,"max_output_tokens":100.9,"input_cost_per_token":0.000002,"output_cost_per_token":0.00001}"#,
        )
        .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0003")));
}

#[tokio::test]
async fn a_text_token_limit_is_read() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":"10"}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 100);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0003")));
}

#[tokio::test]
async fn a_non_numeric_token_limit_falls_back_to_the_default() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":"many"}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.01")));
}

#[tokio::test]
async fn a_null_token_field_falls_through_to_the_next_one() {
    let fixture =
        Fixture::new(r#"{"model":"flat-model","max_completion_tokens":null,"max_tokens":10}"#)
            .model_info("flat-model", FLAT)
            .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.0001")));
}

#[tokio::test]
async fn a_boolean_token_field_reads_as_one() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":true}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.00001")));
}

#[tokio::test]
async fn a_list_token_field_is_not_a_count() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":[10]}"#)
        .model_info("flat-model", FLAT)
        .provided_count("flat-model", 0);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.01")));
}

#[tokio::test]
async fn a_text_encoded_rate_is_read() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":0}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","max_input_tokens":1000,"max_output_tokens":1000,"input_cost_per_token":"0.000002"}"#,
        )
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("0.002")));
}

#[tokio::test]
async fn an_unparsable_rate_is_unknown() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":0}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":"free"}"#,
        )
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn a_null_rate_is_unknown() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":0}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":null}"#,
        )
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn a_list_rate_is_unknown() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":0}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":[0.1]}"#,
        )
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn an_object_rate_is_unknown() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":0}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":{}}"#,
        )
        .provided_count("flat-model", 1000);
    assert_eq!(fixture.max_cost().await.unwrap(), None);
}

#[tokio::test]
async fn a_boolean_rate_reads_as_one() {
    let fixture = Fixture::new(r#"{"model":"flat-model","max_tokens":0}"#)
        .model_info(
            "flat-model",
            r#"{"mode":"chat","max_output_tokens":1000,"input_cost_per_token":true}"#,
        )
        .provided_count("flat-model", 1);
    assert_eq!(fixture.max_cost().await.unwrap(), Some(dec("1")));
}

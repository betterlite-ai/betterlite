#![cfg(test)]

use std::collections::HashSet;
use std::future::{Future, ready};
use std::io;

use rstest::rstest;

use super::custom_fields::CUSTOM_PRICING_FIELDS;
use super::entry::{build_custom_pricing_entry, router_deployment_id};
use super::error::{DeploymentIdError, RequestPricingError};
use super::fields::{
    COST_MAP_LOOKUP_KEY, PRICING_OVERRIDES_KEY, echoed_cost_map_fields,
    echoed_cost_map_pricing_fields, is_server_derived_pricing_key, pricing_override_fields,
    shared_backend_model_info, strip_custom_pricing_fields, without_server_derived_pricing,
};
use super::register::{CustomPricingRequest, register_custom_pricing};
use super::shared_fields::SHARED_MODEL_INFO_FIELDS;
use crate::catalog_generalizations::CatalogGeneralizations;
use crate::model_catalog_keys::ModelCostKeys;
use crate::model_registration::error::RegistrationWarning;
use crate::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelInfoLookup, ModelRegistry,
    RegistrationEnvironment,
};

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn names(fields: &[&str]) -> Vec<String> {
    fields.iter().map(|field| (*field).to_owned()).collect()
}

fn same_name(model: &str) -> String {
    model.to_owned()
}

struct NothingKnown;

impl ModelInfoLookup for NothingKnown {
    fn get_model_info(
        &self,
        _model: &str,
        _model_cost: &JsonObject,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send {
        ready(None)
    }

    fn clear_caches(&self) {}
}

struct NoFetch;

impl CostMapFetcher for NoFetch {
    type Error = io::Error;

    fn get_model_cost_map(
        &self,
        _url: &str,
        _max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, io::Error>> + Send {
        ready(Err(io::Error::other("not used")))
    }
}

async fn register(
    model: &str,
    provider: &str,
    kwargs_text: &str,
    model_info_text: Option<&str>,
) -> Result<(ModelRegistry, Vec<RegistrationWarning>), RequestPricingError<io::Error>> {
    let kwargs = object(kwargs_text);
    let model_info = model_info_text.map(object);
    let generalizations = CatalogGeneralizations::install(Vec::new()).0;
    let environment = RegistrationEnvironment {
        lookup: &NothingKnown,
        fetcher: &NoFetch,
        generalizations: &generalizations,
        bedrock_base_model: same_name,
    };
    let request = CustomPricingRequest {
        model,
        custom_llm_provider: provider,
        kwargs: &kwargs,
        model_info: model_info.as_ref(),
    };
    let mut registry = ModelRegistry::default();
    let registered = register_custom_pricing(&mut registry, &environment, &request).await?;
    Ok((registry, registered.warnings))
}

#[rstest]
#[case::declared_prices_are_copied(
    r#"{"input_cost_per_token":0.001,"cost_per_second":2,"other":1}"#,
    r#"{"litellm_provider":"openai","input_cost_per_token":0.001,"cost_per_second":2}"#
)]
#[case::zero_is_a_real_price(
    r#"{"input_cost_per_token":0,"output_cost_per_second":0.0}"#,
    r#"{"litellm_provider":"openai","input_cost_per_token":0,"output_cost_per_second":0.0}"#
)]
#[case::null_is_skipped(
    r#"{"input_cost_per_token":null,"output_cost_per_token":0.5}"#,
    r#"{"litellm_provider":"openai","output_cost_per_token":0.5}"#
)]
#[case::base_class_fields_count(
    r#"{"output_vector_size":768,"tiered_pricing":[{"range":[0,1]}]}"#,
    r#"{"litellm_provider":"openai","output_vector_size":768,"tiered_pricing":[{"range":[0,1]}]}"#
)]
#[case::nothing_declared("{}", r#"{"litellm_provider":"openai"}"#)]
fn entry_copies_declared_prices(#[case] kwargs: &str, #[case] expected: &str) {
    let entry = build_custom_pricing_entry("openai", &object(kwargs), None);
    assert_eq!(entry, object(expected));
}

#[rstest]
#[case::all_three(
    r#"{"mode":"chat","supports_prompt_caching":true,"max_tokens":0}"#,
    r#"{"litellm_provider":"p","mode":"chat","supports_prompt_caching":true,"max_tokens":0}"#
)]
#[case::null_is_skipped(
    r#"{"mode":null,"supports_prompt_caching":false}"#,
    r#"{"litellm_provider":"p","supports_prompt_caching":false}"#
)]
#[case::other_keys_are_ignored(
    r#"{"id":"x","supports_vision":true}"#,
    r#"{"litellm_provider":"p"}"#
)]
fn entry_fills_metadata_from_model_info(#[case] model_info: &str, #[case] expected: &str) {
    let entry = build_custom_pricing_entry("p", &object("{}"), Some(&object(model_info)));
    assert_eq!(entry, object(expected));
}

#[test]
fn entry_keeps_a_price_field_the_model_info_also_names() {
    let kwargs = object(r#"{"input_cost_per_token":1}"#);
    let model_info = object(r#"{"input_cost_per_token":9,"mode":"embedding"}"#);
    let entry = build_custom_pricing_entry("p", &kwargs, Some(&model_info));
    assert_eq!(
        entry,
        object(r#"{"litellm_provider":"p","input_cost_per_token":1,"mode":"embedding"}"#)
    );
}

#[rstest]
#[case::no_metadata("{}", None)]
#[case::litellm_metadata_first(
    r#"{"litellm_metadata":{"model_info":{"id":"a"}},"metadata":{"model_info":{"id":"b"}}}"#,
    Some("a")
)]
#[case::metadata_when_the_first_has_no_id(
    r#"{"litellm_metadata":{"model_info":{"id":null}},"metadata":{"model_info":{"id":"b"}}}"#,
    Some("b")
)]
#[case::a_non_object_bucket_is_skipped(
    r#"{"litellm_metadata":"text","metadata":{"model_info":{"id":"b"}}}"#,
    Some("b")
)]
#[case::a_null_bucket_is_skipped(
    r#"{"litellm_metadata":null,"metadata":{"model_info":{"id":"b"}}}"#,
    Some("b")
)]
#[case::a_non_object_model_info_is_skipped(
    r#"{"litellm_metadata":{"model_info":[1]},"metadata":{"model_info":{"id":"b"}}}"#,
    Some("b")
)]
#[case::a_bucket_without_model_info_is_skipped(
    r#"{"litellm_metadata":{"other":1},"metadata":{"model_info":{"id":"b"}}}"#,
    Some("b")
)]
#[case::a_number_reads_as_text(r#"{"metadata":{"model_info":{"id":7}}}"#, Some("7"))]
#[case::an_empty_id_is_an_id(r#"{"metadata":{"model_info":{"id":""}}}"#, Some(""))]
#[case::no_id_anywhere(
    r#"{"litellm_metadata":{"model_info":{}},"metadata":{"model_info":{"id":null}}}"#,
    None
)]
fn entry_reads_the_router_deployment_id(#[case] kwargs: &str, #[case] expected: Option<&str>) {
    let found = router_deployment_id(&object(kwargs)).unwrap();
    assert_eq!(found.as_deref(), expected);
}

#[rstest]
#[case::bool("true")]
#[case::list("[1]")]
#[case::object(r#"{"x":1}"#)]
fn entry_rejects_an_id_that_is_not_a_scalar(#[case] id: &str) {
    let kwargs = object(&format!(
        r#"{{"litellm_metadata":{{"model_info":{{"id":{id}}}}}}}"#
    ));
    let error = router_deployment_id(&kwargs).unwrap_err();
    assert!(matches!(
        error,
        DeploymentIdError::NotScalar {
            bucket: "litellm_metadata"
        }
    ));
}

#[tokio::test]
async fn register_a_direct_call_prices_the_shared_key() {
    let (registry, warnings) = register(
        "my-model",
        "openai",
        r#"{"input_cost_per_token":0.001}"#,
        Some(r#"{"mode":"chat"}"#),
    )
    .await
    .unwrap();
    assert_eq!(
        registry.model_cost,
        object(
            r#"{"openai/my-model":{"litellm_provider":"openai","input_cost_per_token":0.001,"mode":"chat"}}"#
        )
    );
    assert!(registry.runtime_registered.is_empty());
    assert_eq!(
        warnings,
        vec![RegistrationWarning::MissingCachePricing {
            model: "openai/my-model".to_owned()
        }]
    );
}

#[tokio::test]
async fn register_a_router_call_keeps_the_prices_under_the_deployment_id() {
    let kwargs = r#"{"input_cost_per_token":0,"metadata":{"model_info":{"id":"dep-1"}}}"#;
    let (registry, warnings) = register("my-model", "openai", kwargs, Some(r#"{"mode":"chat"}"#))
        .await
        .unwrap();
    assert_eq!(
        registry.model_cost,
        object(
            r#"{"dep-1":{"litellm_provider":"openai","input_cost_per_token":0,"mode":"chat"},
                "openai/my-model":{"litellm_provider":"openai","mode":"chat"}}"#
        )
    );
    assert!(registry.runtime_registered.is_empty());
    assert_eq!(
        warnings,
        vec![RegistrationWarning::MissingCachePricing {
            model: "openai/my-model".to_owned()
        }]
    );
}

#[tokio::test]
async fn register_a_numeric_deployment_id_becomes_its_text() {
    let kwargs = r#"{"input_cost_per_token":1,"litellm_metadata":{"model_info":{"id":12}}}"#;
    let (registry, _warnings) = register("m", "azure", kwargs, None).await.unwrap();
    assert!(registry.model_cost.contains_key("12"));
    assert!(registry.model_cost.contains_key("azure/m"));
}

#[tokio::test]
async fn register_refuses_a_deployment_id_that_is_not_a_scalar() {
    let kwargs = r#"{"metadata":{"model_info":{"id":true}}}"#;
    let result = register("m", "azure", kwargs, None).await;
    assert!(matches!(result, Err(RequestPricingError::DeploymentId(_))));
}

#[test]
fn fields_the_custom_list_holds_each_field_once() {
    let unique: HashSet<&&str> = CUSTOM_PRICING_FIELDS.iter().collect();
    assert_eq!(unique.len(), CUSTOM_PRICING_FIELDS.len());
}

#[rstest]
#[case::input_price("input_cost_per_token", true)]
#[case::output_price("output_cost_per_token", true)]
#[case::base_class_tiers("tiered_pricing", true)]
#[case::base_class_vector_size("output_vector_size", true)]
#[case::per_second_rate("input_cost_per_second", true)]
#[case::deployment_scoped_block("off_peak_pricing", false)]
#[case::mode("mode", false)]
fn fields_custom_list_membership(#[case] field: &str, #[case] listed: bool) {
    assert_eq!(CUSTOM_PRICING_FIELDS.contains(&field), listed);
}

#[rstest]
#[case::mode("mode", true)]
#[case::max_tokens("max_tokens", true)]
#[case::lookup_key("key", true)]
#[case::capability_flag("supports_vision", true)]
#[case::price("input_cost_per_token", false)]
#[case::base_class_price("output_vector_size", false)]
#[case::deployment_scoped_block("off_peak_pricing", false)]
#[case::deployment_id("id", false)]
fn fields_shared_list_membership(#[case] field: &str, #[case] listed: bool) {
    assert_eq!(SHARED_MODEL_INFO_FIELDS.contains(&field), listed);
}

#[test]
fn fields_strip_drops_only_the_custom_pricing_fields() {
    let info = object(
        r#"{"input_cost_per_token":1,"output_vector_size":3,"off_peak_pricing":{"a":1},"id":"x","mode":"chat"}"#,
    );
    assert_eq!(
        strip_custom_pricing_fields(&info),
        object(r#"{"off_peak_pricing":{"a":1},"id":"x","mode":"chat"}"#)
    );
}

#[test]
fn fields_shared_info_keeps_only_cost_map_schema_fields() {
    let info = object(
        r#"{"input_cost_per_token":1,"off_peak_pricing":{"a":1},"id":"x","team":"t","mode":"chat","supports_vision":false,"max_tokens":0}"#,
    );
    assert_eq!(
        shared_backend_model_info(&info),
        object(r#"{"mode":"chat","supports_vision":false,"max_tokens":0}"#)
    );
}

#[rstest]
#[case::declared_price("input_cost_per_token", true)]
#[case::declared_second_rate("output_cost_per_second", true)]
#[case::exempt_vector_size("output_vector_size", false)]
#[case::mode("mode", false)]
#[case::lookup_key("key", false)]
#[case::deployment_scoped_block("off_peak_pricing", false)]
#[case::overrides_key("pricing_overrides", false)]
#[case::undeclared_200k_rate("new_rate_above_200k_tokens", true)]
#[case::undeclared_128k_rate("new_rate_above_128k_tokens", true)]
#[case::plain_count("new_rate_above_5_tokens", true)]
#[case::a_k_suffix("new_rate_above_5k_tokens", true)]
#[case::other_script_digits("new_rate_above_\u{0663}\u{0664}k_tokens", true)]
#[case::no_digits("new_rate_above_k_tokens", false)]
#[case::no_digits_and_no_k("new_rate_above__tokens", false)]
#[case::double_k("new_rate_above_5kk_tokens", false)]
#[case::wrong_tail("new_rate_above_5k_token", false)]
#[case::wrong_tail_after_k("new_rate_above_5k", false)]
#[case::no_underscore_before_above("newrateabove_5k_tokens", false)]
#[case::digit_before_the_word("new_rate_above5k_tokens", false)]
#[case::a_letter_inside_the_number("new_rate_above_5a3_tokens", false)]
#[case::interval_rate("input_cost_per_video_per_second_above_8s_interval", true)]
fn fields_server_derived_keys(#[case] key: &str, #[case] expected: bool) {
    assert_eq!(is_server_derived_pricing_key(key), expected);
}

#[test]
fn fields_without_server_derived_pricing_keeps_everything_else() {
    let info = object(
        r#"{"id":"x","mode":"chat","key":"gpt","pricing_overrides":{"a":1},"input_cost_per_token":1,"new_rate_above_9k_tokens":2,"output_vector_size":4,"off_peak_pricing":{"a":1}}"#,
    );
    assert_eq!(
        without_server_derived_pricing(&info),
        object(
            r#"{"id":"x","mode":"chat","key":"gpt","output_vector_size":4,"off_peak_pricing":{"a":1}}"#
        )
    );
    assert_eq!(PRICING_OVERRIDES_KEY, "pricing_overrides");
    assert_eq!(COST_MAP_LOOKUP_KEY, "key");
}

#[rstest]
#[case::needs_the_lookup_key(r#"{"input_cost_per_token":1}"#, &[])]
#[case::sorted_derived_fields(
    r#"{"output_cost_per_token":1,"key":"k","mode":"chat","input_cost_per_token":2,"x_above_1k_tokens":3}"#,
    &["input_cost_per_token", "output_cost_per_token", "x_above_1k_tokens"]
)]
#[case::the_key_alone_counts_for_nothing(r#"{"key":"k","mode":"chat"}"#, &[])]
fn fields_echoed_pricing_needs_the_lookup_key(#[case] info: &str, #[case] expected: &[&str]) {
    assert_eq!(
        echoed_cost_map_pricing_fields(&object(info)),
        names(expected)
    );
}

#[rstest]
#[case::needs_the_lookup_key(r#"{"mode":"chat"}"#, &[r#"{"mode":"chat"}"#], &[])]
#[case::equal_values(
    r#"{"key":"k","mode":"chat","id":"x"}"#,
    &[r#"{"key":"k","mode":"chat"}"#],
    &["key", "mode"]
)]
#[case::edited_value_is_a_real_override(
    r#"{"key":"k","input_cost_per_token":2}"#,
    &[r#"{"key":"k","input_cost_per_token":1}"#],
    &["key"]
)]
#[case::any_entry_may_match(
    r#"{"key":"k","a":1,"b":2}"#,
    &[r#"{"a":1}"#, r#"{"b":2}"#],
    &["a", "b"]
)]
#[case::an_absent_field_never_matches(r#"{"key":"k","a":null}"#, &[r#"{"b":null}"#], &[])]
#[case::null_equals_null(r#"{"key":"k","a":null}"#, &[r#"{"a":null}"#], &["a"])]
#[case::integer_equals_float(
    r#"{"key":"k","a":1,"b":0.5,"c":0}"#,
    &[r#"{"a":1.0,"b":0.50,"c":0.0}"#],
    &["a", "b", "c"]
)]
#[case::exponent_equals_decimal(
    r#"{"key":"k","a":3e-7}"#,
    &[r#"{"a":0.0000003}"#],
    &["a"]
)]
#[case::different_numbers(r#"{"key":"k","a":1}"#, &[r#"{"a":2}"#], &[])]
#[case::numbers_beyond_decimal_range(
    r#"{"key":"k","a":1e40,"b":1e40}"#,
    &[r#"{"a":1e40,"b":2e40}"#],
    &["a"]
)]
#[case::text_is_not_a_number(r#"{"key":"k","a":"1"}"#, &[r#"{"a":1}"#], &[])]
#[case::lists_compare_by_item(
    r#"{"key":"k","a":[1,{"x":2}],"b":[1,2],"c":[1]}"#,
    &[r#"{"a":[1.0,{"x":2.0}],"b":[1],"c":[1,2]}"#],
    &["a"]
)]
#[case::objects_compare_by_field(
    r#"{"key":"k","a":{"x":1,"y":2},"b":{"x":1},"c":{"x":1}}"#,
    &[r#"{"a":{"y":2.0,"x":1.0},"b":{"x":1,"y":2},"c":{"z":1}}"#],
    &["a"]
)]
#[case::no_entries(r#"{"key":"k"}"#, &[], &[])]
fn fields_echoed_values_equal_a_cost_map_entry(
    #[case] info: &str,
    #[case] entries: &[&str],
    #[case] expected: &[&str],
) {
    let parsed: Vec<JsonObject> = entries.iter().map(|entry| object(entry)).collect();
    let references: Vec<&JsonObject> = parsed.iter().collect();
    let found = echoed_cost_map_fields(&object(info), &references);
    assert_eq!(found, names(expected));
}

#[test]
fn fields_override_fields_skip_nulls_and_keep_zero() {
    let first = object(
        r#"{"input_cost_per_token":0,"output_cost_per_token":null,"mode":"chat","z_above_2k_tokens":1}"#,
    );
    let second =
        object(r#"{"input_cost_per_token":5,"output_cost_per_second":0.0,"output_vector_size":3}"#);
    assert_eq!(
        pricing_override_fields(&[&first, &second]),
        names(&[
            "input_cost_per_token",
            "output_cost_per_second",
            "z_above_2k_tokens"
        ])
    );
    assert_eq!(pricing_override_fields(&[]), names(&[]));
}

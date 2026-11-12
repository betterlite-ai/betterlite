#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::PricingInputs;
use super::deployment::{DeploymentReadError, DeploymentTarget};
use super::error::DeploymentPricingError;
use super::inherit::{
    inherit_cache_pricing, inherit_off_peak_rates, inherit_service_tier_pricing,
    inherit_tiered_output_rate,
};
use super::register::{register_deployment_pricing, register_in_cost_map};
use super::replay::replay_deployment_pricing;
use support::{
    COST_MAP, CUSTOM_BACKEND, INPUTS, PTU_TERMS, World, backend, capture_logs, cost_payload,
    deployment, json, merged, object, registry,
};

mod support;

#[tokio::test]
async fn inherit_cache_fills_only_the_unset_cache_fields() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(
        r#"{"id": "a", "cache_read_input_token_cost": 0.5, "cache_creation_input_token_cost": null}"#,
    );
    inherit_cache_pricing(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
    )
    .await;
    assert_eq!(
        info,
        object(
            r#"{"id": "a", "cache_read_input_token_cost": 0.5, "cache_creation_input_token_cost": 0.00000375, "cache_creation_input_token_cost_above_1hr": 0.000006}"#
        )
    );
}

#[tokio::test]
async fn inherit_cache_returns_before_the_lookup_when_all_five_fields_are_set() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let all_five = r#"{
        "cache_creation_input_token_cost": 1,
        "cache_creation_input_token_cost_above_1hr": 2,
        "cache_creation_input_token_cost_above_200k_tokens": 3,
        "cache_read_input_token_cost": 4,
        "cache_read_input_token_cost_above_200k_tokens": 5
    }"#;
    let mut info = object(all_five);
    inherit_cache_pricing(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
    )
    .await;
    assert_eq!(info, object(all_five));
    assert!(world.lookup.queried().is_empty());
}

#[tokio::test]
async fn inherit_cache_looks_up_when_one_of_five_fields_is_missing() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(
        r#"{
            "cache_creation_input_token_cost": 1,
            "cache_creation_input_token_cost_above_1hr": 2,
            "cache_creation_input_token_cost_above_200k_tokens": 3,
            "cache_read_input_token_cost_above_200k_tokens": 5
        }"#,
    );
    inherit_cache_pricing(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
    )
    .await;
    assert_eq!(
        info.get("cache_read_input_token_cost"),
        Some(&json("0.0000003"))
    );
}

#[rstest]
#[case::bare("claude-x", None, "claude-x")]
#[case::provider_added("claude-x", Some("anthropic"), "anthropic/claude-x")]
#[case::provider_already_there("anthropic/claude-x", Some("anthropic"), "anthropic/claude-x")]
#[case::other_prefix("openai/claude-x", Some("anthropic"), "anthropic/openai/claude-x")]
#[tokio::test]
async fn inherit_lookups_name_the_model_with_its_provider(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] queried: &str,
) {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = JsonObject::new();
    inherit_cache_pricing(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend(model, provider),
    )
    .await;
    assert_eq!(world.lookup.queried(), [queried]);
}

#[tokio::test]
async fn inherit_every_step_leaves_the_map_alone_when_the_lookup_finds_nothing() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let start = r#"{
        "input_cost_per_token": 0.5,
        "off_peak_pricing": {"window": "night"},
        "tiered_pricing": [{"range": [0, 1]}]
    }"#;
    let mut info = object(start);
    let unmapped = backend("nope", None);
    inherit_cache_pricing(&mut info, &mut registry, &world.lookup, &unmapped).await;
    inherit_service_tier_pricing(&mut info, &mut registry, &world.lookup, &unmapped, &INPUTS).await;
    let off_peak_only = r#"{"off_peak_pricing": {"window": "night"}}"#;
    let mut schedule = object(off_peak_only);
    inherit_off_peak_rates(&mut schedule, &mut registry, &world.lookup, &unmapped).await;
    inherit_tiered_output_rate(&mut info, &mut registry, &world.lookup, &unmapped).await;
    assert_eq!(info, object(start));
    assert_eq!(schedule, object(off_peak_only));
    assert_eq!(world.lookup.queried(), ["nope", "nope", "nope", "nope"]);
}

#[tokio::test]
async fn inherit_raw_copy_steps_change_nothing_when_the_lookup_result_has_no_key() {
    let mut registry = registry(COST_MAP);
    let mut world = World::new();
    world.lookup.omit_key = true;
    let start = r#"{"input_cost_per_token": 0.5, "off_peak_pricing": {"window": "night"}}"#;
    let mut tiers = object(start);
    inherit_service_tier_pricing(
        &mut tiers,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
        &INPUTS,
    )
    .await;
    let mut off_peak = merged(start, r#"{"input_cost_per_token": null}"#);
    inherit_off_peak_rates(
        &mut off_peak,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
    )
    .await;
    assert_eq!(tiers, object(start));
    assert_eq!(off_peak, merged(start, r#"{"input_cost_per_token": null}"#));
}

#[tokio::test]
async fn inherit_service_tier_copies_only_tier_fields_from_the_raw_entry() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(r#"{"input_cost_per_token": 0.5, "input_cost_per_token_priority": 9}"#);
    inherit_service_tier_pricing(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("claude-x", Some("anthropic")),
        &INPUTS,
    )
    .await;
    assert_eq!(
        info,
        object(
            r#"{"input_cost_per_token": 0.5, "input_cost_per_token_priority": 9, "input_cost_per_token_flex": 0.0000015}"#
        )
    );
}

#[tokio::test]
async fn inherit_service_tier_copies_the_four_suffixes_and_nothing_near_them() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(r#"{"output_cost_per_token": 0.5}"#);
    inherit_service_tier_pricing(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("tiers-all", None),
        &INPUTS,
    )
    .await;
    assert_eq!(
        info,
        object(
            r#"{"output_cost_per_token": 0.5, "a_balanced": 1, "b_flex": 2, "c_priority": 3, "d_ultrafast": 4}"#
        )
    );
}

#[rstest]
#[case::empty("{}", false)]
#[case::both_null(
    r#"{"input_cost_per_token": null, "output_cost_per_token": null}"#,
    false
)]
#[case::input_only(r#"{"input_cost_per_token": 0.5}"#, true)]
#[case::output_only(r#"{"output_cost_per_token": 0.5}"#, true)]
#[tokio::test]
async fn inherit_service_tier_needs_a_base_price(#[case] start: &str, #[case] copies: bool) {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(start);
    inherit_service_tier_pricing(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
        &INPUTS,
    )
    .await;
    assert_eq!(info.contains_key("input_cost_per_token_flex"), copies);
    assert_eq!(world.lookup.queried().len(), usize::from(copies));
}

#[rstest]
#[case::attributed_terms(true, PTU_TERMS, false)]
#[case::attribution_off(false, PTU_TERMS, true)]
#[case::incomplete_terms(true, r#"{"ptu_count": 10, "cost_per_ptu_per_hour": 2}"#, true)]
#[tokio::test]
async fn inherit_service_tier_skips_ptu_deployments_only_with_attribution_on(
    #[case] attribution: bool,
    #[case] terms: &str,
    #[case] copies: bool,
) {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let inputs = PricingInputs {
        ptu_cost_attribution: attribution,
        ..INPUTS
    };
    let mut info = merged(terms, r#"{"input_cost_per_token": 0.5}"#);
    inherit_service_tier_pricing(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
        &inputs,
    )
    .await;
    assert_eq!(info.contains_key("input_cost_per_token_flex"), copies);
}

#[tokio::test]
async fn inherit_off_peak_copies_every_non_null_raw_field_and_keeps_user_fields() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(r#"{"off_peak_pricing": {"window": "night"}, "mode": "embedding"}"#);
    inherit_off_peak_rates(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
    )
    .await;
    assert_eq!(
        info,
        object(
            r#"{
                "off_peak_pricing": {"window": "night"},
                "mode": "embedding",
                "litellm_provider": "anthropic",
                "input_cost_per_token": 0.000003,
                "output_cost_per_token": 0.000015,
                "cache_read_input_token_cost": 0.0000003,
                "cache_creation_input_token_cost": 0.00000375,
                "cache_creation_input_token_cost_above_1hr": 0.000006,
                "input_cost_per_token_flex": 0.0000015,
                "input_cost_per_token_priority": 0.000006,
                "supports_vision": true
            }"#
        )
    );
}

#[tokio::test]
async fn inherit_off_peak_does_not_store_the_zero_rates_the_lookup_makes_up() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(r#"{"off_peak_pricing": [1]}"#);
    inherit_off_peak_rates(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("tiered-only", None),
    )
    .await;
    assert!(info.contains_key("tiered_pricing"));
    assert!(!info.contains_key("input_cost_per_token"));
    assert!(!info.contains_key("output_cost_per_token"));
}

#[rstest]
#[case::missing("{}", false)]
#[case::null(r#"{"off_peak_pricing": null}"#, false)]
#[case::false_flag(r#"{"off_peak_pricing": false}"#, false)]
#[case::empty_list(r#"{"off_peak_pricing": []}"#, false)]
#[case::empty_object(r#"{"off_peak_pricing": {}}"#, false)]
#[case::empty_text(r#"{"off_peak_pricing": ""}"#, false)]
#[case::zero(r#"{"off_peak_pricing": 0}"#, false)]
#[case::zero_float(r#"{"off_peak_pricing": 0.0}"#, false)]
#[case::negative_zero(r#"{"off_peak_pricing": -0.0}"#, false)]
#[case::zero_exponent(r#"{"off_peak_pricing": 0e5}"#, false)]
#[case::true_flag(r#"{"off_peak_pricing": true}"#, true)]
#[case::list(r#"{"off_peak_pricing": [1]}"#, true)]
#[case::object(r#"{"off_peak_pricing": {"a": 1}}"#, true)]
#[case::text(r#"{"off_peak_pricing": "x"}"#, true)]
#[case::number(r#"{"off_peak_pricing": 2}"#, true)]
#[case::small_exponent(r#"{"off_peak_pricing": 1e-7}"#, true)]
#[case::decimal(r#"{"off_peak_pricing": 0.5}"#, true)]
#[tokio::test]
async fn inherit_off_peak_runs_only_for_a_truthy_schedule(
    #[case] start: &str,
    #[case] copies: bool,
) {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(start);
    inherit_off_peak_rates(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("claude-x", None),
    )
    .await;
    assert_eq!(info.contains_key("litellm_provider"), copies);
    assert_eq!(world.lookup.queried().len(), usize::from(copies));
}

#[rstest]
#[case::input_cost("input_cost_per_token", false)]
#[case::input_per_second("input_cost_per_second", false)]
#[case::cost_per_second("cost_per_second", false)]
#[case::tiered_pricing("tiered_pricing", false)]
#[case::output_cost("output_cost_per_token", true)]
#[tokio::test]
async fn inherit_off_peak_skips_when_a_base_price_field_is_set(
    #[case] field: &str,
    #[case] copies: bool,
) {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut set = object(r#"{"off_peak_pricing": true}"#);
    set.insert(field.to_owned(), json("1"));
    let mut null = object(r#"{"off_peak_pricing": true}"#);
    null.insert(field.to_owned(), Json::Null);
    let backend_model = backend("claude-x", None);
    inherit_off_peak_rates(&mut set, &mut registry, &world.lookup, &backend_model).await;
    inherit_off_peak_rates(&mut null, &mut registry, &world.lookup, &backend_model).await;
    assert_eq!(set.contains_key("litellm_provider"), copies);
    assert!(null.contains_key("litellm_provider"));
}

#[tokio::test]
async fn inherit_tiered_output_copies_a_flat_backend_rate() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(r#"{"tiered_pricing": [{"range": [0, 1]}]}"#);
    inherit_tiered_output_rate(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("gpt-x", None),
    )
    .await;
    assert_eq!(info.get("output_cost_per_token"), Some(&json("0.000008")));
    assert_eq!(info.len(), 2);
}

#[rstest]
#[case::no_tiers("{}", false)]
#[case::null_tiers(r#"{"tiered_pricing": null}"#, false)]
#[case::tiers_not_a_list(r#"{"tiered_pricing": "x"}"#, false)]
#[case::empty_tiers(r#"{"tiered_pricing": []}"#, false)]
#[case::output_set(r#"{"tiered_pricing": [{"a": 1}], "output_cost_per_token": 0}"#, false)]
#[case::every_tier_priced(
    r#"{"tiered_pricing": [{"output_cost_per_token": 1}, {"output_cost_per_token": null}]}"#,
    false
)]
#[case::output_null(
    r#"{"tiered_pricing": [{"a": 1}], "output_cost_per_token": null}"#,
    true
)]
#[case::one_tier_unpriced(
    r#"{"tiered_pricing": [{"output_cost_per_token": 1}, {"a": 1}]}"#,
    true
)]
#[case::tier_not_an_object(r#"{"tiered_pricing": [5]}"#, true)]
#[tokio::test]
async fn inherit_tiered_output_keeps_its_early_returns(#[case] start: &str, #[case] copies: bool) {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(start);
    inherit_tiered_output_rate(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend("gpt-x", None),
    )
    .await;
    assert_eq!(
        info.get("output_cost_per_token") == Some(&json("0.000008")),
        copies
    );
    assert_eq!(world.lookup.queried().len(), usize::from(copies));
}

#[rstest]
#[case::synthesized_zero("tiered-only", false)]
#[case::stored_zero_float("zero-rate", false)]
#[case::stored_negative_zero("negative-zero-rate", false)]
#[case::small_rate("tiny-rate", true)]
#[tokio::test]
async fn inherit_tiered_output_never_copies_a_zero_rate(#[case] model: &str, #[case] copies: bool) {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let mut info = object(r#"{"tiered_pricing": [{"a": 1}]}"#);
    inherit_tiered_output_rate(
        &mut info,
        &mut registry,
        &world.lookup,
        &backend(model, None),
    )
    .await;
    assert_eq!(info.contains_key("output_cost_per_token"), copies);
}

#[tokio::test]
async fn payload_folds_non_null_custom_pricing_over_the_model_info() {
    let world = World::new();
    let params = object(
        r#"{"model": "nope", "input_cost_per_token": 0.5, "output_cost_per_token": null, "api_key": "k"}"#,
    );
    let info = object(r#"{"id": "d1", "input_cost_per_token": 9, "output_cost_per_token": 0.7}"#);
    let payload = cost_payload(&world, &params, &info, &INPUTS).await.unwrap();
    assert_eq!(
        payload,
        object(r#"{"id": "d1", "input_cost_per_token": 0.5, "output_cost_per_token": 0.7}"#)
    );
}

#[rstest]
#[case::output_only(r#"{"output_cost_per_token": 0.5}"#, false)]
#[case::input_in_params(r#"{"input_cost_per_token": 0.5}"#, true)]
#[tokio::test]
async fn payload_inherits_cache_pricing_only_with_an_input_price(
    #[case] pricing: &str,
    #[case] inherits: bool,
) {
    let world = World::new();
    let params = merged(r#"{"model": "claude-x"}"#, pricing);
    let info = object(r#"{"id": "d"}"#);
    let payload = cost_payload(&world, &params, &info, &INPUTS).await.unwrap();
    assert_eq!(
        payload.contains_key("cache_read_input_token_cost"),
        inherits
    );
}

#[tokio::test]
async fn payload_counts_an_input_price_that_came_from_the_model_info() {
    let world = World::new();
    let params = object(r#"{"model": "claude-x"}"#);
    let info = object(r#"{"input_cost_per_token": 0.5}"#);
    let payload = cost_payload(&world, &params, &info, &INPUTS).await.unwrap();
    assert!(payload.contains_key("cache_read_input_token_cost"));
}

#[rstest]
#[case::model_info_base_model(r#"{"model": "nope"}"#, r#"{"base_model": "claude-x"}"#, ["nope", "claude-x"], false)]
#[case::params_base_model(r#"{"model": "nope", "base_model": "claude-x"}"#, "{}", ["nope", "claude-x"], false)]
#[case::model_info_wins(
    r#"{"model": "nope", "base_model": "gpt-x"}"#,
    r#"{"base_model": "claude-x"}"#,
    ["nope", "claude-x"],
    false
)]
#[case::own_model(r#"{"model": "claude-x"}"#, "{}", ["claude-x", "claude-x"], true)]
#[tokio::test]
async fn payload_service_tier_follows_the_base_model_and_the_other_steps_do_not(
    #[case] deployment_params: &str,
    #[case] info: &str,
    #[case] queried: [&str; 2],
    #[case] cache_inherited: bool,
) {
    let world = World::new();
    let params = merged(deployment_params, r#"{"input_cost_per_token": 0.5}"#);
    let payload = cost_payload(&world, &params, &object(info), &INPUTS)
        .await
        .unwrap();
    assert_eq!(
        payload.get("input_cost_per_token_flex"),
        Some(&json("0.0000015"))
    );
    assert_eq!(
        payload.contains_key("cache_read_input_token_cost"),
        cache_inherited
    );
    assert_eq!(world.lookup.queried(), queried);
}

#[tokio::test]
async fn payload_looks_models_up_under_the_custom_provider() {
    let world = World::new();
    let params = object(
        r#"{"model": "claude-x", "custom_llm_provider": "anthropic", "input_cost_per_token": 0.5}"#,
    );
    cost_payload(&world, &params, &JsonObject::new(), &INPUTS)
        .await
        .unwrap();
    assert_eq!(world.lookup.queried(), ["anthropic/claude-x"; 2]);
}

#[tokio::test]
async fn payload_gives_an_off_peak_only_deployment_the_backend_entry() {
    let world = World::new();
    let params = object(r#"{"model": "gpt-x"}"#);
    let info = object(r#"{"id": "d", "off_peak_pricing": {"window": "night"}}"#);
    let payload = cost_payload(&world, &params, &info, &INPUTS).await.unwrap();
    assert_eq!(
        payload,
        object(
            r#"{
                "id": "d",
                "off_peak_pricing": {"window": "night"},
                "litellm_provider": "openai",
                "mode": "chat",
                "input_cost_per_token": 0.000002,
                "output_cost_per_token": 0.000008
            }"#
        )
    );
}

#[tokio::test]
async fn payload_fills_a_tiered_deployment_output_rate() {
    let world = World::new();
    let params = object(r#"{"model": "gpt-x", "tiered_pricing": [{"range": [0, 1]}]}"#);
    let payload = cost_payload(&world, &params, &JsonObject::new(), &INPUTS)
        .await
        .unwrap();
    assert_eq!(
        payload,
        object(r#"{"tiered_pricing": [{"range": [0, 1]}], "output_cost_per_token": 0.000008}"#)
    );
}

#[tokio::test]
async fn payload_passes_the_ptu_flag_to_the_service_tier_step() {
    let world = World::new();
    let params = object(r#"{"model": "claude-x", "input_cost_per_token": 0.5}"#);
    let info = object(PTU_TERMS);
    let attributed = PricingInputs {
        ptu_cost_attribution: true,
        ..INPUTS
    };
    let with_flag = cost_payload(&world, &params, &info, &attributed)
        .await
        .unwrap();
    let without_flag = cost_payload(&world, &params, &info, &INPUTS).await.unwrap();
    assert!(!with_flag.contains_key("input_cost_per_token_flex"));
    assert!(without_flag.contains_key("input_cost_per_token_flex"));
}

#[rstest]
#[case::no_model("{}", "{}", "litellm_params.model is missing or not a string")]
#[case::model_not_text(
    r#"{"model": 4}"#,
    "{}",
    "litellm_params.model is missing or not a string"
)]
#[case::provider_not_text(
    r#"{"model": "m", "custom_llm_provider": 4}"#,
    "{}",
    "custom_llm_provider is not a string"
)]
#[case::id_not_text(r#"{"model": "m"}"#, r#"{"id": 4}"#, "id is not a string")]
#[tokio::test]
async fn payload_rejects_a_deployment_it_cannot_read(
    #[case] params: &str,
    #[case] info: &str,
    #[case] message: &str,
) {
    let world = World::new();
    let error = cost_payload(&world, &object(params), &object(info), &INPUTS)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[tokio::test]
async fn register_writes_the_full_entry_under_the_id_and_cost_map_fields_under_the_backend_key() {
    let mut registry = registry("{}");
    let world = World::new();
    let info = object(
        r#"{"id": "dep-1", "db_model": false, "mode": "chat", "supports_vision": true, "input_cost_per_token": 0.5, "custom_key": "x"}"#,
    );
    register_in_cost_map(
        &mut registry,
        &world.environment(),
        &INPUTS,
        &CUSTOM_BACKEND,
        &info,
    )
    .await
    .unwrap();
    assert_eq!(registry.model_cost["dep-1"], Json::Object(info));
    assert_eq!(
        registry.model_cost["acme/custom-m"],
        json(r#"{"mode": "chat", "supports_vision": true}"#)
    );
    assert_eq!(registry.model_cost.len(), 2);
    assert!(registry.runtime_registered.is_empty());
}

#[tokio::test]
async fn register_without_an_id_writes_only_the_shared_entry() {
    let mut registry = registry("{}");
    let world = World::new();
    let target = DeploymentTarget {
        model_id: None,
        ..CUSTOM_BACKEND
    };
    let info = object(r#"{"mode": "chat", "input_cost_per_token": 0.5}"#);
    register_in_cost_map(&mut registry, &world.environment(), &INPUTS, &target, &info)
        .await
        .unwrap();
    assert_eq!(
        registry.model_cost.keys().collect::<Vec<_>>(),
        ["acme/custom-m"]
    );
}

#[tokio::test]
async fn register_without_a_provider_uses_the_bare_model_as_the_backend_key() {
    let mut registry = registry("{}");
    let world = World::new();
    let target = DeploymentTarget {
        custom_llm_provider: None,
        ..CUSTOM_BACKEND
    };
    register_in_cost_map(
        &mut registry,
        &world.environment(),
        &INPUTS,
        &target,
        &object(r#"{"mode": "chat"}"#),
    )
    .await
    .unwrap();
    assert!(registry.model_cost.contains_key("custom-m"));
}

#[tokio::test]
async fn register_a_responses_backend_under_both_of_its_keys() {
    let mut registry = registry("{}");
    let world = World::new();
    let target = DeploymentTarget {
        model_id: None,
        model: "responses/gpt-5",
        custom_llm_provider: Some("openai"),
    };
    register_in_cost_map(
        &mut registry,
        &world.environment(),
        &INPUTS,
        &target,
        &object(r#"{"mode": "responses"}"#),
    )
    .await
    .unwrap();
    let keys = registry.model_cost.keys().collect::<Vec<_>>();
    assert_eq!(keys, ["openai/responses/gpt-5", "openai/gpt-5"]);
    assert_eq!(
        registry.model_cost["openai/gpt-5"],
        json(r#"{"mode": "responses"}"#)
    );
}

#[tokio::test]
async fn register_names_the_model_in_the_missing_cache_pricing_warning() {
    let mut registry = registry("{}");
    let world = World::new();
    let (logs, _guard) = capture_logs();
    let info = object(r#"{"id": "dep-1", "input_cost_per_token": 0.5}"#);
    register_in_cost_map(
        &mut registry,
        &world.environment(),
        &INPUTS,
        &CUSTOM_BACKEND,
        &info,
    )
    .await
    .unwrap();
    let text = logs.text();
    assert_eq!(
        text.matches("register_model: model=custom-m has custom pricing")
            .count(),
        1
    );
    assert!(!text.contains("model=dep-1"));
}

#[tokio::test]
async fn register_a_strategy_router_alias_keeps_no_custom_price_on_either_key() {
    let mut registry = registry("{}");
    let world = World::new();
    let target = DeploymentTarget {
        model_id: Some("alias-1"),
        model: "auto_router/fast",
        custom_llm_provider: None,
    };
    let info = object(
        r#"{"id": "alias-1", "mode": "chat", "input_cost_per_token": 0, "output_cost_per_token": 0, "custom_key": "keep"}"#,
    );
    register_in_cost_map(&mut registry, &world.environment(), &INPUTS, &target, &info)
        .await
        .unwrap();
    assert_eq!(
        registry.model_cost["alias-1"],
        json(r#"{"id": "alias-1", "mode": "chat", "custom_key": "keep"}"#)
    );
    assert_eq!(
        registry.model_cost["auto_router/fast"],
        json(r#"{"mode": "chat"}"#)
    );
}

#[rstest]
#[case::downgrade_kept(Some("responses"), r#""mode": "chat""#, Some("responses"), true)]
#[case::upgrade_applies(Some("chat"), r#""mode": "responses""#, Some("responses"), false)]
#[case::missing_keeps(Some("responses"), "", Some("responses"), false)]
#[case::null_keeps(Some("chat"), r#""mode": null"#, Some("chat"), false)]
#[case::missing_keeps_chat(Some("chat"), "", Some("chat"), false)]
#[case::unrelated_change_applies(
    Some("responses"),
    r#""mode": "embedding""#,
    Some("embedding"),
    false
)]
#[case::first_registration(None, r#""mode": "chat""#, Some("chat"), false)]
#[case::nothing_anywhere(None, "", None, false)]
#[case::same_mode(Some("responses"), r#""mode": "responses""#, Some("responses"), false)]
#[tokio::test]
async fn register_keeps_the_shared_mode_only_for_a_downgrade_or_a_missing_mode(
    #[case] existing: Option<&str>,
    #[case] deployment_mode: &str,
    #[case] expected: Option<&str>,
    #[case] warned: bool,
) {
    let seeded = existing.map_or_else(
        || r#"{"acme/custom-m": {"litellm_provider": "acme"}}"#.to_owned(),
        |mode| format!(r#"{{"acme/custom-m": {{"litellm_provider": "acme", "mode": "{mode}"}}}}"#),
    );
    let mut registry = registry(&seeded);
    let world = World::new();
    let (logs, _guard) = capture_logs();
    let target = DeploymentTarget {
        model_id: None,
        ..CUSTOM_BACKEND
    };
    let info = object(&format!("{{{deployment_mode}}}"));
    register_in_cost_map(&mut registry, &world.environment(), &INPUTS, &target, &info)
        .await
        .unwrap();
    let Json::Object(shared) = &registry.model_cost["acme/custom-m"] else {
        panic!("the shared entry is not an object");
    };
    let mode = match shared.get("mode") {
        Some(Json::Text(text)) => Some(text.as_str()),
        _ => None,
    };
    assert_eq!(mode, expected);
    let warning = "Router: preserving existing mode=responses for shared backend key acme/custom-m instead of the deployment-specified mode=chat (prevents alias registration from downgrading the shared backend mode).";
    assert_eq!(logs.text().contains(warning), warned);
}

#[tokio::test]
async fn register_deployment_pricing_registers_the_payload() {
    let mut registry = registry("{}");
    let world = World::new();
    let params = object(
        r#"{"model": "custom-m", "custom_llm_provider": "acme", "input_cost_per_token": 0.5}"#,
    );
    let info = object(r#"{"id": "dep-9", "mode": "chat"}"#);
    register_deployment_pricing(&mut registry, &params, &info, &world.environment(), &INPUTS)
        .await
        .unwrap();
    assert_eq!(
        registry.model_cost["dep-9"],
        json(r#"{"id": "dep-9", "mode": "chat", "input_cost_per_token": 0.5}"#)
    );
    assert_eq!(
        registry.model_cost["acme/custom-m"],
        json(r#"{"mode": "chat"}"#)
    );
}

#[tokio::test]
async fn register_deployment_pricing_rejects_a_deployment_without_a_model() {
    let mut registry = registry("{}");
    let world = World::new();
    let error = register_deployment_pricing(
        &mut registry,
        &JsonObject::new(),
        &JsonObject::new(),
        &world.environment(),
        &INPUTS,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        DeploymentPricingError::Read(DeploymentReadError::MissingModel)
    ));
    assert!(registry.model_cost.is_empty());
}

#[tokio::test]
async fn register_reports_a_failed_registration_and_stops() {
    let mut registry = registry(COST_MAP);
    let mut world = World::new();
    world.lookup.omit_key = true;
    let target = DeploymentTarget {
        model_id: Some("gpt-x"),
        ..CUSTOM_BACKEND
    };
    let error = register_in_cost_map(
        &mut registry,
        &world.environment(),
        &INPUTS,
        &target,
        &object(r#"{"mode": "chat"}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DeploymentPricingError::Registration(_)));
    assert!(!registry.model_cost.contains_key("acme/custom-m"));
}

#[rstest]
#[case::chat_then_embedding(["chat", "embedding"], "embedding")]
#[case::embedding_then_chat(["embedding", "chat"], "chat")]
#[tokio::test]
async fn replay_registers_every_deployment_in_list_order(
    #[case] modes: [&str; 2],
    #[case] last_wins: &str,
) {
    let mut registry = registry("{}");
    let world = World::new();
    let deployments = [deployment("d1", modes[0]), deployment("d2", modes[1])];
    replay_deployment_pricing(&mut registry, &world.environment(), &deployments, &INPUTS)
        .await
        .unwrap();
    assert!(registry.model_cost.contains_key("d1"));
    assert!(registry.model_cost.contains_key("d2"));
    let Json::Object(shared) = &registry.model_cost["acme/custom-m"] else {
        panic!("the shared entry is not an object");
    };
    assert_eq!(shared["mode"], Json::Text(last_wins.to_owned()));
}

#[tokio::test]
async fn replay_registers_the_payload_with_its_inherited_pricing() {
    let mut registry = registry(COST_MAP);
    let world = World::new();
    let deployments = [json(
        r#"{"litellm_params": {"model": "claude-x", "input_cost_per_token": 0.5}, "model_info": {"id": "d1"}}"#,
    )];
    replay_deployment_pricing(&mut registry, &world.environment(), &deployments, &INPUTS)
        .await
        .unwrap();
    let Json::Object(entry) = &registry.model_cost["d1"] else {
        panic!("the deployment entry is not an object");
    };
    assert_eq!(entry["input_cost_per_token"], json("0.5"));
    assert_eq!(entry["cache_read_input_token_cost"], json("0.0000003"));
}

#[rstest]
#[case::not_an_object("5")]
#[case::no_params(r#"{"model_info": {"id": "bad"}}"#)]
#[case::params_not_an_object(r#"{"litellm_params": "x", "model_info": {"id": "bad"}}"#)]
#[case::no_model(r#"{"litellm_params": {}, "model_info": {"id": "bad"}}"#)]
#[case::no_model_info(r#"{"litellm_params": {"model": "custom-m"}}"#)]
#[case::id_not_text(r#"{"litellm_params": {"model": "custom-m"}, "model_info": {"id": 4}}"#)]
#[tokio::test]
async fn replay_logs_and_skips_a_deployment_it_cannot_read(#[case] unreadable: &str) {
    let mut registry = registry("{}");
    let world = World::new();
    let (logs, _guard) = capture_logs();
    let deployments = [json(unreadable), deployment("good", "chat")];
    replay_deployment_pricing(&mut registry, &world.environment(), &deployments, &INPUTS)
        .await
        .unwrap();
    let text = logs.text();
    assert_eq!(text.matches("ERROR").count(), 1, "{text}");
    assert!(text.contains(
        "Router: could not rebuild cost-map entry for a deployment during a price data reload"
    ));
    assert!(registry.model_cost.contains_key("good"));
    assert!(!registry.model_cost.contains_key("bad"));
}

#[tokio::test]
async fn replay_logs_nothing_when_every_deployment_is_readable() {
    let mut registry = registry("{}");
    let world = World::new();
    let (logs, _guard) = capture_logs();
    replay_deployment_pricing(
        &mut registry,
        &world.environment(),
        &[deployment("a", "chat")],
        &INPUTS,
    )
    .await
    .unwrap();
    assert!(!logs.text().contains("ERROR"));
}

#[tokio::test]
async fn replay_stops_at_a_failed_registration() {
    let mut registry = registry(COST_MAP);
    let mut world = World::new();
    world.lookup.omit_key = true;
    let deployments = [deployment("gpt-x", "chat"), deployment("later", "chat")];
    let error =
        replay_deployment_pricing(&mut registry, &world.environment(), &deployments, &INPUTS)
            .await
            .unwrap_err();
    assert!(matches!(error, DeploymentPricingError::Registration(_)));
    assert!(!registry.model_cost.contains_key("later"));
}

#[tokio::test]
async fn replay_of_no_deployments_registers_nothing() {
    let mut registry = registry("{}");
    let world = World::new();
    replay_deployment_pricing(&mut registry, &world.environment(), &[], &INPUTS)
        .await
        .unwrap();
    assert!(registry.model_cost.is_empty());
}

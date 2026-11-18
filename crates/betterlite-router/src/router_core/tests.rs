#![cfg(test)]

mod behaviors;
mod boundaries;
pub(crate) mod support;

use std::future::{Future, Ready, ready};

use betterlite_models::decisions::Json;
use rstest::rstest;

use self::support::{Fixture, RequestSetup, run_route, seeded, with_info};
use crate::context::RouteStage;
use crate::generalization_integration::model_generalizations::ModelGeneralizations;
use crate::router_core::services::RouteServices;
use crate::strategy_dispatch::strategy::RoutingStrategy;

// A synchronous case, as the completed future the case table awaits.
fn settled(case: fn()) -> Ready<()> {
    case();
    ready(())
}

// a routing rule and a capability rule over the same backend model, as a cost map or a config names them
const GENERALIZATION_RULES: &str = r#"[{"pattern":"^openai/","model_info":{"litellm_provider":"acme"}},{"pattern":"^openai/","model_info":{"supports_vision":true,"max_tokens":100}}]"#;

// a router that carries installed rules
struct Generalized(ModelGeneralizations);

impl RouteServices for Generalized {
    fn model_generalizations(&self) -> Option<&ModelGeneralizations> {
        Some(&self.0)
    }
}

// the rules fill a deployment named by its id too, as get_model_info fills every registered deployment
async fn deployment_id_generalized() {
    let named = with_info(
        "gpt-4o",
        "a",
        &[],
        &[("supports_vision", Json::Bool(false))],
    );
    let services =
        Generalized(ModelGeneralizations::install(GENERALIZATION_RULES.as_bytes()).unwrap());
    let mut random = seeded();
    let result = run_route(
        &Fixture::plain(vec![named]),
        &RequestSetup::new("a"),
        &services,
        RoutingStrategy::SimpleShuffle,
        &mut random,
    )
    .await
    .unwrap();
    assert_eq!(
        result.deployment.model_info.get("litellm_provider"),
        Some(&Json::Text(String::from("acme")))
    );
    assert_eq!(
        result.deployment.model_info.get("supports_vision"),
        Some(&Json::Bool(false))
    );
    assert!(matches!(
        result.deployment.model_info.get("max_tokens"),
        Some(Json::Number(number)) if number.as_u64() == Some(100)
    ));
}

#[rstest]
#[case::weighted_pick(behaviors::weighted_pick())]
#[case::shared_pool_chat_and_embedding(behaviors::shared_pool_chat_and_embedding())]
#[case::callback_narrowed(behaviors::callback_narrowed())]
#[case::specific_deployment_pool(behaviors::specific_deployment_pool())]
#[case::pinned_short_circuit(behaviors::pinned_short_circuit())]
#[case::deployment_id_single(behaviors::deployment_id_single())]
#[case::deployment_id_generalized(deployment_id_generalized())]
#[case::excluded_drop(behaviors::excluded_drop())]
#[case::retry_skip_narrow(behaviors::retry_skip_narrow())]
#[case::retry_skip_keeps_pool(behaviors::retry_skip_keeps_pool())]
#[case::group_alias(behaviors::group_alias())]
#[case::model_alias_map(behaviors::model_alias_map())]
#[case::client_acquired(behaviors::client_acquired())]
#[case::healthy_pool(behaviors::healthy_pool())]
#[case::healthy_single(behaviors::healthy_single())]
#[case::healthy_blocked_pool(behaviors::healthy_blocked_pool())]
#[case::order_lowest_wins(behaviors::order_lowest_wins())]
#[case::order_after_blocked(behaviors::order_after_blocked())]
#[case::headers_attach_to_object(settled(behaviors::headers_attach_to_object))]
#[case::header_model_id_kept(settled(behaviors::header_model_id_kept))]
#[case::token_count_from_usage(settled(behaviors::token_count_from_usage))]
#[case::token_count_from_input_output(settled(behaviors::token_count_from_input_output))]
#[case::override_pre_call_bound(behaviors::override_pre_call_bound())]
#[case::override_pre_call_absent(behaviors::override_pre_call_absent())]
#[tokio::test]
async fn router_core_behaviors<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    case.await;
}

#[rstest]
#[case::stage_cooldown_policy(boundaries::stage_skips(RouteStage::CooldownPolicy))]
#[case::stage_fallback_attempts(boundaries::stage_skips(RouteStage::FallbackAttempts))]
#[case::stage_chat_stream_fallback(boundaries::stage_skips(RouteStage::ChatStreamFallback))]
#[case::stage_native_stream_fallback(boundaries::stage_skips(RouteStage::NativeStreamFallback))]
#[case::stage_model_rate_limits(boundaries::stage_skips(RouteStage::ModelRateLimits))]
#[case::stage_tag_routing(boundaries::stage_skips(RouteStage::TagRouting))]
#[case::stage_access_windows(boundaries::stage_skips(RouteStage::AccessWindows))]
#[case::routing_group_priority_reaches_the_pick(
    boundaries::routing_group_priority_reaches_the_pick()
)]
#[case::routing_group_members_missing_from_the_pool_leave_none(
    boundaries::routing_group_members_missing_from_the_pool_leave_none()
)]
#[case::stage_deployment_affinity(boundaries::stage_skips(RouteStage::DeploymentAffinity))]
#[case::stage_request_scheduler(boundaries::stage_skips(RouteStage::RequestScheduler))]
#[case::stage_routing_plugins(boundaries::stage_skips(RouteStage::RoutingPlugins))]
#[case::stage_responses_stream_fallback(boundaries::stage_skips(
    RouteStage::ResponsesStreamFallback
))]
#[case::least_busy(boundaries::strategy_unbuilt(RoutingStrategy::LeastBusy, "least_busy"))]
#[case::latency_routing(boundaries::strategy_unbuilt(
    RoutingStrategy::LatencyBasedRouting,
    "latency_routing"
))]
#[case::cost_routing(boundaries::strategy_unbuilt(
    RoutingStrategy::CostBasedRouting,
    "cost_routing"
))]
#[case::usage_routing(boundaries::strategy_unbuilt(
    RoutingStrategy::UsageBasedRouting,
    "usage_routing"
))]
#[case::usage_routing_v2(boundaries::strategy_unbuilt(
    RoutingStrategy::UsageBasedRoutingV2,
    "usage_routing_v2"
))]
#[case::no_deployment_rate_limit(boundaries::no_deployment_rate_limit())]
#[case::no_deployment_all_in_cooldown(boundaries::no_deployment_all_in_cooldown())]
#[case::default_min_cooldown(settled(boundaries::default_min_cooldown))]
#[case::cancelled_callback(boundaries::cancelled_callback())]
#[case::prompt_affinity_narrows_the_pool(boundaries::prompt_affinity_narrows_the_pool())]
#[case::prompt_affinity_before_pre_call_checks(boundaries::prompt_affinity_before_pre_call_checks())]
#[case::empty_group(boundaries::empty_group())]
#[case::blocked_deployment(boundaries::blocked_deployment())]
#[case::blocked_pool(boundaries::blocked_pool())]
#[case::blocked_pool_empty(boundaries::blocked_pool_empty())]
#[case::healthy_catalog_error(boundaries::healthy_catalog_error())]
#[case::no_deployment_infinite_cooldown(boundaries::no_deployment_infinite_cooldown())]
#[case::context_window_after_blocked(boundaries::context_window_after_blocked())]
#[case::pre_call_disabled_keeps_over_window(boundaries::pre_call_disabled_keeps_over_window())]
#[case::compaction_keeps_over_window(boundaries::compaction_keeps_over_window())]
#[case::pre_call_uncountable_skips(boundaries::pre_call_uncountable_skips())]
#[case::response_format_empties_pool(boundaries::response_format_empties_pool())]
#[case::window_keeps_fitting_beside_rejected(boundaries::window_keeps_fitting_beside_rejected())]
#[case::environment_filtered_single(boundaries::environment_filtered_single())]
#[case::environment_filtered_pool(boundaries::environment_filtered_pool())]
#[case::web_search_filtered_pool(boundaries::web_search_filtered_pool())]
#[case::pinned_but_empty(boundaries::pinned_but_empty())]
#[case::pinned_pool_uses_strategy(boundaries::pinned_pool_uses_strategy())]
#[case::provider_budget(boundaries::provider_budget_has_no_selection())]
#[case::simple_shuffle_empty(boundaries::simple_shuffle_empty())]
#[case::weights_read_metrics(settled(boundaries::weights_read_metrics))]
#[case::headers_absent(settled(boundaries::headers_absent))]
#[case::headers_opaque(settled(boundaries::headers_opaque))]
#[case::headers_empty_model_id(settled(boundaries::headers_empty_model_id))]
#[case::header_empty_metadata_id(settled(boundaries::header_empty_metadata_id))]
#[case::headers_non_text_model_id(settled(boundaries::headers_non_text_model_id))]
#[case::headers_non_text_model_id_kept(settled(boundaries::headers_non_text_model_id_kept))]
#[case::headers_non_object_map(settled(boundaries::headers_non_object_map))]
#[case::header_dict_reads_existing(settled(boundaries::header_dict_reads_existing))]
#[case::token_count_absent_usage(settled(boundaries::token_count_absent_usage))]
#[case::token_count_zero_total(settled(boundaries::token_count_zero_total))]
#[case::token_count_non_numeric(settled(boundaries::token_count_non_numeric))]
#[tokio::test]
async fn router_core_boundaries<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    case.await;
}

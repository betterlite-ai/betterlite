use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::support::{
    Cancelling, Checked, Cooldown, Fixture, Providers, RequestSetup, Windows, body, deployment,
    error_of, group_and_ids, hidden, hidden_with, metadata, no_deployment_of, number, object,
    paused, picked, run_checked, run_healthy, run_route, run_simple, run_windowed, seeded, text,
    weighted, with_info,
};
use crate::context::{self, RouteStage};
use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::candidates::{CandidateSet, weights};
use crate::router_core::error::RoutingError;
use crate::router_core::headers::{
    Response, apply_response_model_id, ensure_additional_headers, ensure_additional_headers_dict,
    prepare_response, response_has_hidden_params, response_total_token_count, write_hidden_params,
};
use crate::router_core::no_deployment::{NoDeployment, NoDeploymentType};
use crate::router_core::services::RouteServices as _;
use crate::router_core::tests::support::{
    NoLimits, NoServices, route_context, route_request, strategy_context,
};
use crate::simple_shuffle::Candidate;
use crate::simple_shuffle::error::ShuffleError;
use crate::static_eligibility::error::EligibilityError;
use crate::static_eligibility::request::RequestFacts;
use crate::strategy::{self, Strategy};
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, ResolutionSource};

pub(super) async fn run(name: &str) {
    if cooldown_stage_cases(name).await {
        return;
    }
    if routing_stage_cases(name).await {
        return;
    }
    if strategy_cases(name).await {
        return;
    }
    if no_deployment_cases(name).await {
        return;
    }
    if filter_cases(name).await {
        return;
    }
    response_cases(name).await;
}

async fn cooldown_stage_cases(name: &str) -> bool {
    match name {
        "stage_cooldown_state" => stage_skips(RouteStage::CooldownState).await,
        "stage_cooldown_policy" => stage_skips(RouteStage::CooldownPolicy).await,
        "stage_fallback_attempts" => stage_skips(RouteStage::FallbackAttempts).await,
        "stage_chat_stream_fallback" => stage_skips(RouteStage::ChatStreamFallback).await,
        "stage_native_stream_fallback" => stage_skips(RouteStage::NativeStreamFallback).await,
        "stage_generalization_integration" => {
            stage_skips(RouteStage::GeneralizationIntegration).await;
        }
        _ => return false,
    }
    true
}

async fn routing_stage_cases(name: &str) -> bool {
    match name {
        "stage_model_rate_limits" => stage_skips(RouteStage::ModelRateLimits).await,
        "stage_tag_routing" => stage_skips(RouteStage::TagRouting).await,
        "stage_access_windows" => stage_skips(RouteStage::AccessWindows).await,
        "stage_routing_groups" => stage_skips(RouteStage::RoutingGroups).await,
        "stage_deployment_affinity" => stage_skips(RouteStage::DeploymentAffinity).await,
        "stage_prompt_affinity" => stage_skips(RouteStage::PromptAffinity).await,
        "stage_request_scheduler" => stage_skips(RouteStage::RequestScheduler).await,
        "stage_routing_plugins" => stage_skips(RouteStage::RoutingPlugins).await,
        "stage_responses_stream_fallback" => stage_skips(RouteStage::ResponsesStreamFallback).await,
        _ => return false,
    }
    true
}

async fn strategy_cases(name: &str) -> bool {
    match name {
        "least_busy" => strategy_unbuilt(RoutingStrategy::LeastBusy, "least_busy").await,
        "latency_routing" => {
            strategy_unbuilt(RoutingStrategy::LatencyBasedRouting, "latency_routing").await;
        }
        "cost_routing" => strategy_unbuilt(RoutingStrategy::CostBasedRouting, "cost_routing").await,
        "usage_routing" => {
            strategy_unbuilt(RoutingStrategy::UsageBasedRouting, "usage_routing").await;
        }
        "usage_routing_v2" => {
            strategy_unbuilt(RoutingStrategy::UsageBasedRoutingV2, "usage_routing_v2").await;
        }
        "lar1_routing" => strategy_unbuilt(RoutingStrategy::Lar1, "lar1_routing").await,
        _ => return false,
    }
    true
}

async fn no_deployment_cases(name: &str) -> bool {
    match name {
        "no_deployment_rate_limit" => no_deployment_rate_limit().await,
        "no_deployment_all_in_cooldown" => no_deployment_all_in_cooldown().await,
        "default_min_cooldown" => default_min_cooldown(),
        "cancelled_callback" => cancelled_callback().await,
        "empty_group" => empty_group().await,
        "blocked_deployment" => blocked_deployment().await,
        "blocked_pool" => blocked_pool().await,
        "blocked_pool_empty" => blocked_pool_empty().await,
        _ => return false,
    }
    true
}

async fn filter_cases(name: &str) -> bool {
    match name {
        "healthy_catalog_error" => healthy_catalog_error().await,
        "environment_filtered_single" => environment_filtered_single().await,
        "environment_filtered_pool" => environment_filtered_pool().await,
        "web_search_filtered_pool" => web_search_filtered_pool().await,
        "pinned_but_empty" => pinned_but_empty().await,
        "pinned_pool_uses_strategy" => pinned_pool_uses_strategy().await,
        "provider_budget" => provider_budget_has_no_selection().await,
        "simple_shuffle_empty" => simple_shuffle_empty().await,
        _ => return false,
    }
    true
}

async fn response_cases(name: &str) {
    match name {
        "weights_read_metrics" => weights_read_metrics(),
        "headers_absent" => headers_absent(),
        "headers_opaque" => headers_opaque(),
        "headers_empty_model_id" => headers_empty_model_id(),
        "headers_non_text_model_id" => headers_non_text_model_id(),
        "headers_non_text_model_id_kept" => headers_non_text_model_id_kept(),
        "headers_non_object_map" => headers_non_object_map(),
        "header_dict_reads_existing" => header_dict_reads_existing(),
        "token_count_absent_usage" => token_count_absent_usage(),
        "token_count_zero_total" => token_count_zero_total(),
        "token_count_non_numeric" => token_count_non_numeric(),
        "header_empty_metadata_id" => header_empty_metadata_id(),
        "no_deployment_infinite_cooldown" => no_deployment_infinite_cooldown().await,
        "context_window_after_blocked" => context_window_after_blocked().await,
        "pre_call_disabled_keeps_over_window" => pre_call_disabled_keeps_over_window().await,
        "compaction_keeps_over_window" => compaction_keeps_over_window().await,
        "pre_call_uncountable_skips" => pre_call_uncountable_skips().await,
        "response_format_empties_pool" => response_format_empties_pool().await,
        "window_keeps_fitting_beside_rejected" => window_keeps_fitting_beside_rejected().await,
        _ => panic!("unknown case {name}"),
    }
}

// the entry each stub module names, as its module file spells it
fn stage_name(stage: RouteStage) -> &'static str {
    match stage {
        RouteStage::CooldownState => "cooldown_state",
        RouteStage::CooldownPolicy => "cooldown_policy",
        RouteStage::FallbackAttempts => "fallback_attempts",
        RouteStage::ChatStreamFallback => "chat_stream_fallback",
        RouteStage::NativeStreamFallback => "native_stream_fallback",
        RouteStage::GeneralizationIntegration => "generalization_integration",
        RouteStage::ModelRateLimits => "model_rate_limits",
        RouteStage::TagRouting => "tag_routing",
        RouteStage::AccessWindows => "access_windows",
        RouteStage::RoutingGroups => "routing_groups",
        RouteStage::DeploymentAffinity => "deployment_affinity",
        RouteStage::PromptAffinity => "prompt_affinity",
        RouteStage::RequestScheduler => "request_scheduler",
        RouteStage::RoutingPlugins => "routing_plugins",
        RouteStage::ResponsesStreamFallback => "responses_stream_fallback",
    }
}

async fn stage_skips(stage: RouteStage) {
    let setup = RequestSetup::new("gpt-4o");
    let request = setup.request();
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let context = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("gpt-4o", "a", &[])],
    };
    let skipped = context::run(stage, set.clone(), &context).await.unwrap();
    assert_eq!(skipped, set);
    let error = context::apply(stage, set, &context).await.unwrap_err();
    assert_eq!(error.unbuilt_entry(), Some(stage_name(stage)));
}

async fn strategy_unbuilt(strategy: RoutingStrategy, entry: &'static str) {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let mut random = seeded();
    let error = run_route(
        &fixture,
        &RequestSetup::new("gpt-4o"),
        &NoServices,
        strategy,
        &mut random,
    )
    .await
    .unwrap_err();
    assert_eq!(error.unbuilt_entry(), Some(entry));
}

async fn no_deployment_rate_limit() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]));
    let mut setup = RequestSetup::new("gpt-4o");
    setup.excluded = vec![String::from("a"), String::from("b")];
    let error = run_simple(&fixture, &setup).await.unwrap_err();
    let RoutingError::NoDeployment(no_deployment) = error else {
        panic!("an empty pool reports no deployment");
    };
    assert_eq!(no_deployment.error_type, NoDeploymentType::RateLimit);
    assert_eq!(no_deployment.error_type.wire_name(), "rate_limit_error");
    assert_eq!(no_deployment.model, "gpt-4o");
    assert_eq!(no_deployment.cooldown_time.to_bits(), 5.0_f64.to_bits());
    assert!(!no_deployment.enable_pre_call_checks);
    assert!(no_deployment.cooldown_list.is_empty());
    assert_eq!(
        no_deployment.model_ids,
        vec![String::from("a"), String::from("b")]
    );
    assert_eq!(
        no_deployment.to_string(),
        "No deployments available for selected model, Try again in 5.0 seconds. Passed model=gpt-4o. pre-call-checks=false, cooldown_list=[]"
    );
}

async fn no_deployment_all_in_cooldown() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]));
    let services = Cooldown {
        ids: vec![String::from("a"), String::from("b")],
        seconds: 2.5,
    };
    let mut setup = RequestSetup::new("gpt-4o");
    setup.excluded = vec![String::from("a"), String::from("b")];
    let mut random = seeded();
    let error = run_route(
        &fixture,
        &setup,
        &services,
        RoutingStrategy::SimpleShuffle,
        &mut random,
    )
    .await
    .unwrap_err();
    let RoutingError::NoDeployment(no_deployment) = error else {
        panic!("an empty pool reports no deployment");
    };
    assert_eq!(no_deployment.error_type, NoDeploymentType::AllInCooldown);
    assert_eq!(
        no_deployment.error_type.wire_name(),
        "all_deployments_in_cooldown"
    );
    assert_eq!(no_deployment.cooldown_time.to_bits(), 2.5_f64.to_bits());
    assert_eq!(
        no_deployment.cooldown_list,
        vec![String::from("a"), String::from("b")]
    );
    assert!(
        no_deployment
            .to_string()
            .contains(" All deployments for selected model are in cooldown.")
    );
}

fn default_min_cooldown() {
    assert_eq!(
        NoServices.min_cooldown(&[String::from("a")]).to_bits(),
        5.0_f64.to_bits()
    );
}

async fn cancelled_callback() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a")]));
    let mut random = seeded();
    let error = run_route(
        &fixture,
        &RequestSetup::new("gpt-4o"),
        &Cancelling,
        RoutingStrategy::SimpleShuffle,
        &mut random,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RoutingError::Cancelled));
    assert!(error.unbuilt_entry().is_none());
}

async fn empty_group() {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let error = error_of(&fixture, &RequestSetup::new("missing")).await;
    let RoutingError::Catalog(CatalogLookupError::NoHealthyDeployments { model }) = error else {
        panic!("a group with no deployment is a catalog error");
    };
    assert_eq!(model, "missing");
}

async fn blocked_deployment() {
    let fixture = Fixture::plain(vec![paused("gpt-4o", "a", &[])]);
    let error = error_of(&fixture, &RequestSetup::new("a")).await;
    let RoutingError::Eligibility(EligibilityError::Blocked { model }) = error else {
        panic!("a paused deployment cannot serve a request");
    };
    assert_eq!(model, "openai/a");
}

// a paused deployment in a pool never serves the request, whatever its weight says
async fn blocked_pool() {
    let fixture = Fixture::plain(vec![
        paused("gpt-4o", "paused", &[("weight", number(5))]),
        deployment("gpt-4o", "live", &[]),
    ]);
    let result = run_simple(&fixture, &RequestSetup::new("gpt-4o"))
        .await
        .unwrap();
    assert_eq!(picked(&result), "live");
}

async fn blocked_pool_empty() {
    let fixture = Fixture::plain(vec![paused("gpt-4o", "paused", &[])]);
    let no_deployment = no_deployment_of(&fixture, &RequestSetup::new("gpt-4o")).await;
    assert_eq!(no_deployment.model, "gpt-4o");
    assert_eq!(no_deployment.model_ids, vec![String::from("paused")]);
}

// a cooldown the float cannot narrow is reported as itself, never as a zero-second retry
async fn no_deployment_infinite_cooldown() {
    let fixture = Fixture::plain(vec![paused("gpt-4o", "paused", &[])]);
    let services = Cooldown {
        ids: Vec::new(),
        seconds: f64::INFINITY,
    };
    let mut random = seeded();
    let error = run_route(
        &fixture,
        &RequestSetup::new("gpt-4o"),
        &services,
        RoutingStrategy::SimpleShuffle,
        &mut random,
    )
    .await
    .unwrap_err();
    let RoutingError::NoDeployment(no_deployment) = error else {
        panic!("a pool of paused deployments leaves nothing to select");
    };
    assert!(
        no_deployment
            .to_string()
            .contains("Try again in inf seconds.")
    );
}

// the context window runs after the paused filter: a paused deployment that fits cannot hide the live one that does not
async fn context_window_after_blocked() {
    let fitting = paused("gpt-4o", "a", &[("order", number(1))]);
    let small = deployment("gpt-4o", "b", &[("order", number(2))]);
    let fixture = Fixture::plain(vec![fitting, small]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.facts.messages = Some(Vec::new());
    setup.facts.input_token_count = Some(50);
    let windows = Windows {
        by_id: BTreeMap::from([(String::from("a"), 100), (String::from("b"), 10)]),
    };
    let checks = Checked {
        limits: &windows,
        params: &Providers::Unknown,
        pre_call: true,
    };
    let error = run_checked(&fixture, &setup, &NoServices, &checks)
        .await
        .unwrap_err();
    let RoutingError::Eligibility(EligibilityError::ContextWindow(exceeded)) = error else {
        panic!("every live deployment exceeds the context window");
    };
    assert!(
        exceeded.causes.contains("Max Input Tokens=10"),
        "{}",
        exceeded.causes
    );
    assert!(exceeded.causes.contains("Got=50"), "{}", exceeded.causes);
}

// An over-window request against the one deployment whose ten-token window cannot hold its fifty tokens.
fn tight_window() -> (Fixture, RequestSetup, Windows) {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.facts.messages = Some(Vec::new());
    setup.facts.input_token_count = Some(50);
    let windows = Windows {
        by_id: BTreeMap::from([(String::from("a"), 10)]),
    };
    (fixture, setup, windows)
}

// the pre-call checks are the router's opt-in: with them off the window never drops a deployment
async fn pre_call_disabled_keeps_over_window() {
    let (fixture, setup, windows) = tight_window();
    let result = run_windowed(&fixture, &setup, &windows, false)
        .await
        .unwrap();
    assert_eq!(picked(&result), "a");
}

// a request under compaction skips the window count, so the too-small deployment still serves
async fn compaction_keeps_over_window() {
    let (fixture, mut setup, windows) = tight_window();
    setup.facts.compaction_pending = true;
    let result = run_windowed(&fixture, &setup, &windows, true)
        .await
        .unwrap();
    assert_eq!(picked(&result), "a");
}

// a request that carries neither messages nor input has nothing to count, so the window check is skipped
async fn pre_call_uncountable_skips() {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.facts.input_token_count = Some(50);
    let windows = Windows {
        by_id: BTreeMap::from([(String::from("a"), 10)]),
    };
    let result = run_windowed(&fixture, &setup, &windows, true)
        .await
        .unwrap();
    assert_eq!(picked(&result), "a");
}

// the supported-parameter filter empties a provider pool without a window rejection, which is the no-deployment answer
async fn response_format_empties_pool() {
    let fixture = Fixture::plain(vec![
        deployment("gpt-4o", "a", &[]),
        deployment("gpt-4o", "b", &[]),
    ]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.facts.messages = Some(Vec::new());
    setup.facts.response_format = true;
    let checks = Checked {
        limits: &NoLimits,
        params: &Providers::OpenAi,
        pre_call: true,
    };
    let error = run_checked(&fixture, &setup, &NoServices, &checks)
        .await
        .unwrap_err();
    assert!(matches!(error, RoutingError::NoDeployment(_)));
}

// a window rejection beside a deployment that fits keeps the fitting one serving
async fn window_keeps_fitting_beside_rejected() {
    let fixture = Fixture::plain(vec![
        deployment("gpt-4o", "small", &[]),
        deployment("gpt-4o", "big", &[]),
    ]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.facts.messages = Some(Vec::new());
    setup.facts.input_token_count = Some(50);
    let windows = Windows {
        by_id: BTreeMap::from([(String::from("small"), 10), (String::from("big"), 100)]),
    };
    let checks = Checked {
        limits: &windows,
        params: &Providers::Unknown,
        pre_call: true,
    };
    let result = run_checked(&fixture, &setup, &NoServices, &checks)
        .await
        .unwrap();
    assert_eq!(picked(&result), "big");
}

// the typed healthy view returns the catalog error LiteLLM's legacy helper swallows
async fn healthy_catalog_error() {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let setup = RequestSetup::new("missing");
    let error = run_healthy(&fixture, &setup).await.unwrap_err();
    assert!(matches!(
        error,
        RoutingError::Catalog(CatalogLookupError::NoHealthyDeployments { model }) if model == "missing"
    ));
}

fn in_production(id: &str) -> DeploymentSpec {
    with_info(
        "gpt-4o",
        id,
        &[],
        &[(
            "supported_environments",
            Json::List(vec![text("production")]),
        )],
    )
}

// The no-deployment error a production-only deployment answers a development request with.
async fn environment_filtered(name: &str) -> NoDeployment {
    let fixture = Fixture::plain(vec![in_production("a")]);
    let mut setup = RequestSetup::new(name);
    setup.environment = Some(String::from("development"));
    no_deployment_of(&fixture, &setup).await
}

async fn environment_filtered_single() {
    let no_deployment = environment_filtered("a").await;
    assert_eq!(no_deployment.model, "openai/a");
    assert_eq!(no_deployment.model_ids, vec![String::from("a")]);
}

async fn environment_filtered_pool() {
    let no_deployment = environment_filtered("gpt-4o").await;
    assert_eq!(no_deployment.model, "gpt-4o");
    assert_eq!(no_deployment.model_ids, vec![String::from("a")]);
}

// a request that asks for web search leaves only the deployments that declare support for it
async fn web_search_filtered_pool() {
    let unsupported = with_info(
        "gpt-4o",
        "a",
        &[("weight", number(5))],
        &[("supports_web_search", Json::Bool(false))],
    );
    let fixture = Fixture::plain(vec![unsupported, deployment("gpt-4o", "b", &[])]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.facts.tools = vec![Json::Object(object(&[("type", text("web_search"))]))];
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(picked(&result), "b");
}

async fn pinned_but_empty() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a")]));
    let mut setup = RequestSetup::new("gpt-4o");
    setup.pinned = true;
    setup.excluded = vec![String::from("a")];
    let error = run_simple(&fixture, &setup).await.unwrap_err();
    assert!(matches!(&error, RoutingError::NoDeployment(_)));
}

// a pin over a pool of two narrows nothing by itself, so the strategy still picks
async fn pinned_pool_uses_strategy() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 0), weighted("gpt-4o", "b", 5)]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.pinned = true;
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(picked(&result), "b");
    assert_eq!(result.strategy, Some(Strategy::SimpleShuffle));
}

async fn provider_budget_has_no_selection() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a")]));
    let mut random = seeded();
    let error = run_route(
        &fixture,
        &RequestSetup::new("gpt-4o"),
        &NoServices,
        RoutingStrategy::ProviderBudgetRouting,
        &mut random,
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, RoutingError::NoDeployment(_)));
    assert!(error.unbuilt_entry().is_none());
}

// the strategy answers even a pool the filters emptied before it ran
async fn simple_shuffle_empty() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::SimpleShuffle, None).unwrap();
    let resolution = Resolution {
        strategy: RoutingStrategy::SimpleShuffle,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    };
    let deployments: [DeploymentSpec; 0] = [];
    let context = strategy_context(&deployments, &route, &mut dispatch, &resolution);
    let mut random = seeded();
    let error = strategy::select(Strategy::SimpleShuffle, context, &mut random)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RoutingError::Shuffle {
            source: ShuffleError::NoCandidates
        }
    ));
}

// LiteLLM reads weight, rpm and tpm as numbers, an absent or null one as zero, and names anything else
fn weights_read_metrics() {
    numeric_weights();
    non_numeric_weights();
    out_of_range_weights();
}

fn numeric_weights() {
    let numeric = vec![
        deployment(
            "gpt-4o",
            "a",
            &[
                ("weight", number(5)),
                ("rpm", number(3)),
                ("tpm", number(7)),
            ],
        ),
        deployment("gpt-4o", "b", &[]),
        deployment("gpt-4o", "c", &[("tpm", Json::Null)]),
    ];
    assert_eq!(
        weights(&numeric).unwrap(),
        vec![
            Candidate {
                weight: 5,
                rpm: 3,
                tpm: 7,
            },
            Candidate {
                weight: 0,
                rpm: 0,
                tpm: 0,
            },
            Candidate {
                weight: 0,
                rpm: 0,
                tpm: 0,
            },
        ]
    );
}

// a metric that is not a number is named by the field it was read from
fn non_numeric_weights() {
    let text_weight = vec![deployment("gpt-4o", "d", &[("weight", text("5"))])];
    assert!(matches!(
        weights(&text_weight),
        Err(RoutingError::Metric { field: "weight" })
    ));
    let fractional_rpm = vec![deployment(
        "gpt-4o",
        "e",
        &[(
            "rpm",
            Json::Number(serde_json::Number::from_f64(0.5).expect("0.5")),
        )],
    )];
    assert!(matches!(
        weights(&fractional_rpm),
        Err(RoutingError::Metric { field: "rpm" })
    ));
}

fn out_of_range_weights() {
    let negative_weight = vec![deployment(
        "gpt-4o",
        "f",
        &[("weight", Json::Number(serde_json::Number::from(-1)))],
    )];
    assert!(matches!(
        weights(&negative_weight),
        Err(RoutingError::Metric { field: "weight" })
    ));
    let huge_tpm = vec![deployment(
        "gpt-4o",
        "g",
        &[(
            "tpm",
            Json::Number(serde_json::Number::from(u64::from(u32::MAX) + 1)),
        )],
    )];
    assert!(matches!(
        weights(&huge_tpm),
        Err(RoutingError::Metric { field: "tpm" })
    ));
}

fn headers_absent() {
    assert_eq!(prepare_response(Response::Absent), None);
}

fn headers_opaque() {
    let mut prepared = prepare_response(Response::Opaque).unwrap();
    assert!(!response_has_hidden_params(&prepared));
    ensure_additional_headers(&mut prepared).insert(String::from("x-probe"), text("1"));
    apply_response_model_id(&mut prepared, Some(&metadata("a")));
    assert!(!response_has_hidden_params(&prepared));
    assert_eq!(write_hidden_params(prepared), Response::Opaque);
}

// The response a request's model id is applied to: a body already carrying one stored model id and the probe header.
fn applied_response(stored: Json) -> Response {
    let mut prepared = prepare_response(body(&[hidden(&[("model_id", stored)])])).unwrap();
    ensure_additional_headers(&mut prepared).insert(String::from("x-probe"), text("1"));
    apply_response_model_id(&mut prepared, Some(&metadata("a")));
    write_hidden_params(prepared)
}

fn headers_empty_model_id() {
    assert_eq!(
        applied_response(text("")),
        Response::Object(hidden_with(text("a")))
    );
}

// a request metadata id that is empty names no deployment, so the response gets no model_id
fn header_empty_metadata_id() {
    let mut prepared = prepare_response(body(&[("id", text("chatcmpl-1"))])).unwrap();
    ensure_additional_headers(&mut prepared);
    apply_response_model_id(&mut prepared, Some(&metadata("")));
    assert_eq!(
        write_hidden_params(prepared),
        body(&[
            ("id", text("chatcmpl-1")),
            (
                "_hidden_params",
                Json::Object(object(&[(
                    "additional_headers",
                    Json::Object(JsonObject::new()),
                )])),
            ),
        ])
    );
}

// a stored model id that is not text is replaced by the request's id too
fn headers_non_text_model_id() {
    assert_eq!(
        applied_response(number(5)),
        Response::Object(hidden_with(text("a")))
    );
}

fn headers_non_text_model_id_kept() {
    let mut prepared = prepare_response(body(&[hidden(&[("model_id", number(5))])])).unwrap();
    ensure_additional_headers(&mut prepared).insert(String::from("x-probe"), text("1"));
    assert_eq!(
        write_hidden_params(prepared),
        Response::Object(hidden_with(number(5)))
    );
}

fn headers_non_object_map() {
    let mut prepared = prepare_response(body(&[hidden(&[
        ("additional_headers", text("text")),
        ("model_id", text("a")),
        ("custom", number(3)),
    ])]))
    .unwrap();
    ensure_additional_headers(&mut prepared).insert(String::from("x-probe"), text("1"));
    let expected = object(&[(
        "_hidden_params",
        Json::Object(object(&[
            ("custom", number(3)),
            ("model_id", text("a")),
            (
                "additional_headers",
                Json::Object(object(&[("x-probe", text("1"))])),
            ),
        ])),
    )]);
    assert_eq!(write_hidden_params(prepared), Response::Object(expected));
}

fn header_dict_reads_existing() {
    let existing = object(&[(
        "additional_headers",
        Json::Object(object(&[("a-header", text("v"))])),
    )]);
    assert_eq!(
        ensure_additional_headers_dict(&existing),
        object(&[("a-header", text("v"))])
    );
    assert_eq!(
        ensure_additional_headers_dict(&object(&[("additional_headers", text("text"))])),
        JsonObject::new()
    );
    assert_eq!(
        ensure_additional_headers_dict(&object(&[])),
        JsonObject::new()
    );
}

fn token_count_absent_usage() {
    assert_eq!(response_total_token_count(&object(&[])), 0);
    assert_eq!(
        response_total_token_count(&object(&[("usage", text("usage"))])),
        0
    );
}

fn token_count_zero_total() {
    let response = object(&[(
        "usage",
        Json::Object(object(&[
            ("total_tokens", number(0)),
            ("input_tokens", number(4)),
            ("output_tokens", number(5)),
        ])),
    )]);
    assert_eq!(response_total_token_count(&response), 9);
}

fn token_count_non_numeric() {
    let response = object(&[(
        "usage",
        Json::Object(object(&[("total_tokens", text("9"))])),
    )]);
    assert_eq!(response_total_token_count(&response), 0);
}

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_anthropic::count_tokens::handler::AnthropicCountTokensHandler;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::anthropic_count_tokens::DEFAULT_BETA_VERSION;
use betterlite_config::sections::cache_cost_routing as section;
use betterlite_models::decisions::Json;
use betterlite_router::cache_cost_routing::{CacheAwareChoice, PreRoutingResponse};
use betterlite_router::complexity_config::checks::Feature;
use betterlite_router::router_predicates::JsonMap;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::routing::cache_alternatives::error::CacheAlternativesError;
use crate::routing::cache_alternatives::host::CacheRoutingHost as _;
use crate::routing::cache_alternatives::selection::{SelectionRequest, select_cached_model};
use crate::routing::cache_alternatives::tests::support::{
    CALLER, CHEAP_ID, CHEAP_MODEL, FakeHost, FakeTierParams, Fixture, NOW, Observation, Rates,
    STRONG_ID, STRONG_MODEL, ScriptedTransport, body, body_with_max_tokens, caller, caller_key,
    decimal, decision, json, ladder, request_kwargs, response, seed, seed_observation,
    strong_deployment, tier_limits,
};

const CONTEXT: &str = "You are a helpful assistant.";
const BIGGER_ID: &str = "test-cache-bigger";

fn selection_case(request_body: &Json) -> SelectionCase {
    SelectionCase {
        config: section::Config {
            enabled: true,
            output_tokens: 1024,
            timeout_ms: 2000,
        },
        response: response("SIMPLE", "cheap"),
        body: request_body.clone(),
        request_kwargs: request_kwargs(CALLER, request_body),
        caller: caller(CALLER),
        now: Some(NOW),
        params: FakeTierParams::default(),
    }
}

// select_cached_model's arguments for one case, gathered so a test changes only the part it is about.
struct SelectionCase {
    config: section::Config,
    response: PreRoutingResponse,
    body: Json,
    request_kwargs: JsonMap,
    caller: UserAPIKeyAuth,
    now: Option<f64>,
    params: FakeTierParams,
}

impl SelectionCase {
    async fn run(
        &self,
        host: &FakeHost<'_>,
    ) -> Result<Option<CacheAwareChoice>, CacheAlternativesError> {
        select_cached_model(&SelectionRequest {
            host,
            config: self.config,
            routing: host.routing_config(),
            params_for_model: &self.params,
            response: &self.response,
            request_kwargs: &self.request_kwargs,
            messages: None,
            body: &self.body,
            caller: &self.caller,
            now: self.now,
        })
        .await
    }
}

fn count_handler(
    total: Option<i64>,
    prefix: Option<i64>,
) -> AnthropicCountTokensHandler<ScriptedTransport> {
    AnthropicCountTokensHandler {
        transport: ScriptedTransport::new(total, prefix),
        request_timeout: Duration::from_secs(5),
        beta_version: DEFAULT_BETA_VERSION.to_owned(),
    }
}

fn strong_observation(body: &Json) -> Observation<'_> {
    Observation {
        deployment: STRONG_ID,
        model: STRONG_MODEL,
        body,
        tokens: 50_000,
        observed_at: 990.0,
        expires_at: 1290.0,
    }
}

#[tokio::test]
async fn a_warm_stronger_model_wins_with_its_exact_costs() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let choice = case
        .run(&host)
        .await
        .unwrap()
        .expect("the warm alternative wins");
    assert_eq!(choice.model, "strong");
    assert_eq!(choice.tier, "COMPLEX");
    assert_eq!(choice.deployment_id, STRONG_ID);
    assert_eq!(choice.original_cost, decimal("0.067596"));
    assert_eq!(choice.estimated_cost, decimal("0.03336"));
    assert_eq!(host.requests(), 4);
    assert_eq!(
        host.authorization_calls(),
        vec!["cheap".to_owned(), "strong".to_owned()]
    );
}

#[tokio::test]
async fn a_higher_output_price_can_outweigh_the_cache_discount() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    fixture.cost.set(
        STRONG_MODEL,
        Rates {
            output: decimal("0.0001"),
            ..Rates::strong()
        },
    );
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 4);
}

#[tokio::test]
async fn a_missing_observation_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
}

#[tokio::test]
async fn an_expired_observation_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 999.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
}

#[tokio::test]
async fn an_observation_for_another_caller_does_not_count() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed_observation(
        &fixture.cache,
        &caller_key("another-caller"),
        &strong_observation(&case.body),
    );
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn a_caller_without_a_key_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let mut case = selection_case(&body(CONTEXT));
    seed_observation(
        &fixture.cache,
        &caller_key(""),
        &strong_observation(&case.body),
    );
    case.caller = caller("");
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
}

#[tokio::test]
async fn an_observation_for_another_prefix_does_not_count() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    let other = body("A completely different system prompt.");
    seed_observation(
        &fixture.cache,
        &caller_key(CALLER),
        &strong_observation(&other),
    );
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn an_unauthorized_alternative_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.authorized.retain(|name| name.as_str() != "strong");
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert_eq!(
        host.authorization_calls(),
        vec!["cheap".to_owned(), "strong".to_owned()]
    );
}

#[tokio::test]
async fn a_disabled_setting_reads_nothing() {
    let fixture = Fixture::new();
    let mut case = selection_case(&body(CONTEXT));
    case.config.enabled = false;
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
}

#[tokio::test]
async fn a_pinned_policy_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.features = vec![Feature::Adaptive, Feature::SessionAffinity];
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
}

#[rstest]
#[case::total_missing(None, Some(50_000), 2)]
#[case::prefix_missing(Some(51_000), None, 4)]
#[case::prefix_over_total(Some(1_000), Some(50_000), 4)]
#[tokio::test]
async fn unavailable_or_inconsistent_counts_keep_the_classified_model(
    #[case] total: Option<i64>,
    #[case] prefix: Option<i64>,
    #[case] requests: usize,
) {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.handler = count_handler(total, prefix);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), requests);
}

#[tokio::test]
async fn the_output_cap_uses_the_smaller_of_setting_and_limit() {
    let fixture = Fixture::new();
    let mut case = selection_case(&body_with_max_tokens(CONTEXT, 1));
    case.config.output_tokens = 100_000;
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let choice = case.run(&host).await.unwrap().expect("one output token");
    assert_eq!(choice.original_cost, decimal("0.063504"));
    assert_eq!(choice.estimated_cost, decimal("0.018015"));
}

#[tokio::test]
async fn an_estimate_that_cannot_fit_is_not_selected() {
    let fixture = Fixture::new();
    let case = selection_case(&body_with_max_tokens(CONTEXT, 60_000));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 4);
}

#[tokio::test]
async fn equal_costs_keep_the_classified_model() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let free = Rates {
        input: Decimal::ZERO,
        output: Decimal::ZERO,
        read: Decimal::ZERO,
        creation: Decimal::ZERO,
    };
    fixture.cost.set(CHEAP_MODEL, free);
    fixture.cost.set(STRONG_MODEL, free);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 4);
}

#[rstest]
#[case::heuristic_scorer("heuristic_scorer")]
#[case::heuristic_v2("heuristic_v2")]
#[case::reasoning_override("reasoning_override")]
#[case::llm_classifier("llm_classifier")]
#[case::llm_v2_classifier("llm_v2_classifier")]
#[case::jev_classifier("jev_classifier")]
#[case::capability_classifier("capability_classifier")]
#[case::heuristic_first_short_circuit("heuristic_first_short_circuit")]
#[case::hybrid_short_circuit("hybrid_short_circuit")]
#[case::classifier_plugin("classifier_plugin")]
#[tokio::test]
async fn every_classified_cause_may_switch_models(#[case] cause: &str) {
    let fixture = Fixture::new();
    let first = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &first.body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let mut case = selection_case(&body(CONTEXT));
    case.response.routing_decision = Some(decision(&[("tier", "SIMPLE"), ("cause", cause)]));
    let choice = case
        .run(&host)
        .await
        .unwrap()
        .expect("a classified cause may switch");
    assert_eq!(choice.model, "strong");
}

#[tokio::test]
async fn each_model_uses_its_own_tier_max_tokens() {
    let fixture = Fixture::new();
    let mut case = selection_case(&body(CONTEXT));
    case.params = tier_limits(&[("SIMPLE", "cheap", 50), ("COMPLEX", "strong", 100)]);
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let choice = case
        .run(&host)
        .await
        .unwrap()
        .expect("the capped alternative wins");
    assert_eq!(choice.original_cost, decimal("0.0637"));
    assert_eq!(choice.estimated_cost, decimal("0.0195"));
}

#[rstest]
#[case::zero(0)]
#[case::negative(-5)]
#[tokio::test]
async fn a_non_positive_tier_limit_stops_the_comparison(#[case] limit: i64) {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let mut attempt = selection_case(&body(CONTEXT));
    attempt.params = tier_limits(&[("COMPLEX", "strong", limit)]);
    assert!(attempt.run(&host).await.unwrap().is_none(), "limit {limit}");
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn authorization_is_settled_before_any_provider_count() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.authorized.retain(|name| name.as_str() != "strong");
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert_eq!(
        host.authorization_calls(),
        vec!["cheap".to_owned(), "strong".to_owned()]
    );
}

#[tokio::test]
async fn an_unauthorized_original_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.authorized.retain(|name| name.as_str() != "cheap");
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert_eq!(
        host.authorization_calls(),
        vec!["cheap".to_owned(), "strong".to_owned()]
    );
}

#[tokio::test]
async fn evidence_that_expires_during_the_counts_is_dropped() {
    let fixture = Fixture::new();
    let mut case = selection_case(&body(CONTEXT));
    case.now = None;
    seed(&fixture.cache, &case.body, 1000.5);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.clock = Mutex::new(VecDeque::from([NOW, NOW + 0.5_f64]));
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 4);
}

#[tokio::test]
async fn evidence_still_fresh_at_the_second_reading_switches() {
    let fixture = Fixture::new();
    let mut case = selection_case(&body(CONTEXT));
    case.now = None;
    seed(&fixture.cache, &case.body, 1000.4);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.clock = Mutex::new(VecDeque::from([NOW, NOW + 0.3_f64]));
    let choice = case.run(&host).await.unwrap().expect("still fresh");
    assert_eq!(choice.estimated_cost, decimal("0.03336"));
}

#[tokio::test]
async fn an_alternative_lookup_error_is_returned() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.missing.push("strong".to_owned());
    let error = case.run(&host).await.unwrap_err();
    assert!(matches!(error, CacheAlternativesError::Deployments { .. }));
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn an_alternative_with_a_failed_health_lookup_is_dropped() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.unhealthy.push("strong".to_owned());
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn a_body_without_a_cache_breakpoint_reads_nothing() {
    let fixture = Fixture::new();
    let plain = json(r#"{"max_tokens":20000,"messages":[{"role":"user","content":"hello"}]}"#);
    let case = selection_case(&plain);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
}

#[tokio::test]
async fn the_classified_model_must_be_eligible() {
    let fixture = Fixture::new();
    let mut case = selection_case(&body(CONTEXT));
    case.response = response("SIMPLE", "cold");
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn a_lower_tier_never_competes() {
    let fixture = Fixture::new();
    let mut case = selection_case(&body(CONTEXT));
    case.response = response("COMPLEX", "strong");
    let observation = Observation {
        deployment: CHEAP_ID,
        model: CHEAP_MODEL,
        body: &case.body,
        tokens: 50_000,
        observed_at: 990.0,
        expires_at: 1290.0,
    };
    seed_observation(&fixture.cache, &caller_key(CALLER), &observation);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
}

#[tokio::test]
async fn an_absent_original_deployment_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    assert!(host.deployments.remove("cheap").is_some());
    assert!(case.run(&host).await.unwrap().is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn the_only_eligible_tier_still_reaches_the_candidate_lookup() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.tiers = ladder(&[("SIMPLE", "cheap")]);
    host.missing.push("cheap".to_owned());
    let error = case.run(&host).await.unwrap_err();
    assert!(matches!(error, CacheAlternativesError::Deployments { .. }));
}

// a second alternative above strong: the same underlying model, its own deployment and a higher tier
fn add_bigger_deployment(host: &mut FakeHost<'_>) {
    let mut bigger = strong_deployment(30_000);
    bigger.model_name = "bigger".to_owned();
    bigger
        .model_info
        .insert("id".to_owned(), Json::Text(BIGGER_ID.to_owned()));
    host.deployments.insert("bigger".to_owned(), vec![bigger]);
    host.authorized.push("bigger".to_owned());
    host.tiers = ladder(&[
        ("SIMPLE", "cheap"),
        ("COMPLEX", "strong"),
        ("REASONING", "bigger"),
    ]);
}

#[tokio::test]
async fn a_cost_tie_between_alternatives_keeps_the_earlier_tier() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    seed_observation(
        &fixture.cache,
        &caller_key(CALLER),
        &Observation {
            deployment: BIGGER_ID,
            model: STRONG_MODEL,
            body: &case.body,
            tokens: 50_000,
            observed_at: 990.0,
            expires_at: 1290.0,
        },
    );
    let context = fixture.context();
    let mut host = fixture.host(&context);
    add_bigger_deployment(&mut host);
    let choice = case
        .run(&host)
        .await
        .unwrap()
        .expect("the tie keeps the earlier tier");
    assert_eq!(choice.model, "strong");
    assert_eq!(choice.tier, "COMPLEX");
    assert_eq!(choice.deployment_id, STRONG_ID);
    assert_eq!(choice.estimated_cost, decimal("0.03336"));
}

#[tokio::test]
async fn a_cheaper_later_alternative_replaces_the_earlier_one() {
    let fixture = Fixture::new();
    let case = selection_case(&body(CONTEXT));
    seed(&fixture.cache, &case.body, 1290.0);
    seed_observation(
        &fixture.cache,
        &caller_key(CALLER),
        &Observation {
            deployment: BIGGER_ID,
            model: STRONG_MODEL,
            body: &case.body,
            tokens: 51_000,
            observed_at: 990.0,
            expires_at: 1290.0,
        },
    );
    let context = fixture.context();
    let mut host = fixture.host(&context);
    add_bigger_deployment(&mut host);
    let choice = case
        .run(&host)
        .await
        .unwrap()
        .expect("the cheaper alternative wins");
    assert_eq!(choice.model, "bigger");
    assert_eq!(choice.tier, "REASONING");
    assert_eq!(choice.deployment_id, BIGGER_ID);
    assert_eq!(choice.estimated_cost, decimal("0.03066"));
}

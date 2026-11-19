#![cfg(test)]

use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::ArmRequest;
use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::tests::support::BODY;
use crate::cache_aware_routing::tests::support::CALLER;
use crate::cache_aware_routing::tests::support::DEPLOYMENT_ID;
use crate::cache_aware_routing::tests::support::FakeCost;
use crate::cache_aware_routing::tests::support::FakeMinimum;
use crate::cache_aware_routing::tests::support::MODEL;
use crate::cache_aware_routing::tests::support::MULTI_BODY;
use crate::cache_aware_routing::tests::support::MemoryCache;
use crate::cache_aware_routing::tests::support::ONE_HOUR_BODY;
use crate::cache_aware_routing::tests::support::PARAMS;
use crate::cache_aware_routing::tests::support::RecordingMinimum;
use crate::cache_aware_routing::tests::support::Resolution;
use crate::cache_aware_routing::tests::support::ScriptedCounter;
use crate::cache_aware_routing::tests::support::billing_time;
use crate::cache_aware_routing::tests::support::json;
use crate::cache_aware_routing::tests::support::model_info;
use crate::cache_aware_routing::tests::support::observation_json;
use crate::cache_aware_routing::tests::support::observation_key;
use crate::cache_aware_routing::tests::support::params;
use crate::cache_aware_routing::tests::support::prediction_services;
use crate::cache_aware_routing::tests::support::scope;
use crate::cache_aware_routing::tests::support::settings;
use crate::cache_aware_routing::types::CacheEvidenceConfidence;
use crate::cache_aware_routing::types::CacheEvidenceSource;
use crate::cache_aware_routing::types::CachePredictionArm;
use crate::cache_aware_routing::types::CacheState;
use crate::cache_aware_routing::types::TokenCountSource;
use crate::cache_plan::PromptPrefix;
use crate::credentials::AnthropicSettings;

const CHECKED_AT: f64 = 1_700_000_000.0;
const FRESH_UNTIL: f64 = 1_700_000_060.0;
const EXPIRED_AT: f64 = 1_699_999_940.0;
const LARGE_TOTAL: i64 = 100;
const SMALL_PREFIX: i64 = 40;

struct Fixture {
    cache: MemoryCache,
    cost: FakeCost,
    minimum: FakeMinimum,
    settings: AnthropicSettings,
    resolution: Resolution,
    litellm_params: JsonObject,
    info: JsonObject,
    checked_at: f64,
}

impl Fixture {
    fn new() -> Self {
        Self {
            cache: MemoryCache::new(),
            cost: FakeCost::new(Some(Decimal::ONE)),
            minimum: FakeMinimum { tokens: 0 },
            settings: settings("https://api.anthropic.com"),
            resolution: Resolution::new(),
            litellm_params: params(PARAMS),
            info: model_info(),
            checked_at: CHECKED_AT,
        }
    }

    fn seed(&self, fingerprint: &str, cached_tokens: i64, expires_at: f64) {
        let key = observation_key(&scope(DEPLOYMENT_ID, MODEL), fingerprint);
        self.cache.seed(
            &key,
            observation_json(fingerprint, cached_tokens, CHECKED_AT, expires_at),
        );
    }

    async fn predict(&self, body: &str, replies: &[Option<i64>]) -> CachePredictionArm {
        let routing = AnthropicCacheRouting::from_body(&json(body)).expect("a predictable body");
        let deployment = PredictionDeployment::read(&self.litellm_params, &self.info);
        let context = self.resolution.context();
        let services = prediction_services(
            &self.settings,
            &context,
            &self.cache,
            &self.cost,
            &self.minimum,
        );
        let counter = ScriptedCounter::new(replies);
        let request = ArmRequest {
            deployment,
            caller_key_hash: CALLER,
            checked_at: self.checked_at,
        };
        routing.predict(&request, &counter, &services).await
    }
}

fn prefix(body: &str) -> PromptPrefix {
    AnthropicCacheRouting::from_body(&json(body))
        .expect("a predictable body")
        .prefix
}

fn counts(total: i64, prefix_tokens: i64) -> Vec<Option<i64>> {
    vec![Some(total), Some(prefix_tokens)]
}

#[tokio::test]
async fn an_unobserved_prefix_is_unknown_and_prices_three_scenarios() {
    let fixture = Fixture::new();
    let arm = fixture
        .predict(BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(arm.deployment_id, DEPLOYMENT_ID);
    assert_eq!(arm.model.as_deref(), Some(MODEL));
    assert_eq!(arm.cache_state, CacheState::Unknown);
    assert_eq!(arm.reason.as_deref(), Some("no_compatible_observation"));
    assert!(arm.evidence.is_none());
    assert_eq!(
        arm.token_count_source,
        Some(TokenCountSource::AnthropicCountTokens)
    );
    let estimate = arm.estimate.clone().expect("an estimate");
    assert_eq!(estimate.tokens.uncached_input_tokens, 60);
    assert_eq!(estimate.tokens.cache_creation_5m_input_tokens, SMALL_PREFIX);
    assert_eq!(estimate.tokens.cache_read_input_tokens, 0);
    assert_eq!(arm.cold.as_ref(), Some(&estimate));
    let warm = arm.warm.clone().expect("a warm scenario");
    assert_eq!(warm.tokens.cache_read_input_tokens, SMALL_PREFIX);
    assert_eq!(warm.tokens.cache_creation_5m_input_tokens, 0);
    let calls = fixture.cost.calls();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|call| call.moment == billing_time()));
    assert!(calls.iter().all(|call| call.model == MODEL));
    assert!(calls.iter().all(|call| call.deployment_id == DEPLOYMENT_ID));
    assert_eq!(calls[2].tokens, estimate.tokens);
    assert_eq!(calls[1].tokens, warm.tokens);
}

#[tokio::test]
async fn a_fresh_exact_observation_is_warm() {
    let fixture = Fixture::new();
    let fingerprint = prefix(BODY).fingerprint;
    fixture.seed(&fingerprint, 80, FRESH_UNTIL);
    let arm = fixture
        .predict(BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(arm.cache_state, CacheState::Warm);
    assert_eq!(arm.reason, None);
    let evidence = arm.evidence.expect("evidence");
    assert_eq!(evidence.observed_at, CHECKED_AT);
    assert_eq!(evidence.expires_at, FRESH_UNTIL);
    assert_eq!(evidence.source, CacheEvidenceSource::ProviderUsage);
    assert_eq!(evidence.confidence, CacheEvidenceConfidence::Observed);
    let estimate = arm.estimate.expect("an estimate");
    assert_eq!(estimate.tokens.cache_read_input_tokens, 80);
    assert_eq!(estimate.tokens.cache_creation_5m_input_tokens, 0);
    assert_eq!(estimate.tokens.uncached_input_tokens, 20);
    let cold = arm.cold.expect("a cold scenario");
    assert_eq!(cold.tokens.cache_creation_5m_input_tokens, 80);
    assert_eq!(cold.tokens.cache_read_input_tokens, 0);
    assert_eq!(cold.tokens.uncached_input_tokens, 20);
}

#[tokio::test]
async fn a_fresh_lookback_observation_is_partial() {
    let fixture = Fixture::new();
    let prefix = prefix(MULTI_BODY);
    assert!(prefix.fingerprints.len() > 1);
    fixture.seed(&prefix.fingerprints[1], 30, FRESH_UNTIL);
    let arm = fixture
        .predict(MULTI_BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(arm.cache_state, CacheState::Partial);
    assert_eq!(arm.reason, None);
    assert!(arm.evidence.is_some());
    let estimate = arm.estimate.expect("an estimate");
    assert_eq!(estimate.tokens.cache_read_input_tokens, 30);
    assert_eq!(estimate.tokens.cache_creation_5m_input_tokens, 10);
    assert_eq!(estimate.tokens.uncached_input_tokens, 60);
    let warm = arm.warm.expect("a warm scenario");
    assert_eq!(warm.tokens.cache_read_input_tokens, SMALL_PREFIX);
}

#[tokio::test]
async fn an_expired_exact_observation_is_stale() {
    let fixture = Fixture::new();
    let fingerprint = prefix(BODY).fingerprint;
    fixture.seed(&fingerprint, 80, EXPIRED_AT);
    let arm = fixture
        .predict(BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(arm.cache_state, CacheState::Stale);
    assert_eq!(arm.reason.as_deref(), Some("observation_expired"));
    let evidence = arm.evidence.expect("evidence");
    assert_eq!(evidence.expires_at, EXPIRED_AT);
    let estimate = arm.estimate.expect("an estimate");
    assert_eq!(estimate.tokens.cache_read_input_tokens, 0);
    assert_eq!(estimate.tokens.cache_creation_5m_input_tokens, 80);
}

#[tokio::test]
async fn an_observation_expiring_exactly_at_the_check_is_stale() {
    let fixture = Fixture::new();
    let fingerprint = prefix(BODY).fingerprint;
    fixture.seed(&fingerprint, 80, CHECKED_AT);
    let arm = fixture
        .predict(BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(arm.cache_state, CacheState::Stale);
    assert_eq!(arm.reason.as_deref(), Some("observation_expired"));
    assert_eq!(
        arm.estimate
            .expect("an estimate")
            .tokens
            .cache_read_input_tokens,
        0
    );
}

#[rstest]
#[case::the_total(&[None])]
#[case::the_prefix(&[Some(10), None])]
#[case::a_shorter_prefix_count(&[Some(10), Some(20)])]
#[tokio::test]
async fn an_unavailable_count_has_no_scenarios(#[case] replies: &[Option<i64>]) {
    let fixture = Fixture::new();
    let arm = fixture.predict(BODY, replies).await;
    assert_eq!(arm.cache_state, CacheState::Unknown);
    assert_eq!(arm.reason.as_deref(), Some("token_count_unavailable"));
    assert!(arm.estimate.is_none());
    assert!(arm.cold.is_none());
    assert!(arm.warm.is_none());
    assert!(arm.token_count_source.is_none());
    assert!(fixture.cost.calls().is_empty());
}

#[tokio::test]
async fn an_exact_observation_above_the_total_is_inconsistent() {
    let fixture = Fixture::new();
    let fingerprint = prefix(BODY).fingerprint;
    fixture.seed(&fingerprint, 150, FRESH_UNTIL);
    let arm = fixture
        .predict(BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(
        arm.reason.as_deref(),
        Some("inconsistent_prefix_token_count")
    );
    assert_eq!(arm.cache_state, CacheState::Unknown);
    assert!(arm.estimate.is_none());
    assert!(fixture.cost.calls().is_empty());
}

#[tokio::test]
async fn a_lookback_observation_above_the_prefix_is_inconsistent() {
    let fixture = Fixture::new();
    let prefix = prefix(MULTI_BODY);
    fixture.seed(&prefix.fingerprints[1], 50, FRESH_UNTIL);
    let arm = fixture
        .predict(MULTI_BODY, &counts(200, SMALL_PREFIX))
        .await;
    assert_eq!(
        arm.reason.as_deref(),
        Some("inconsistent_prefix_token_count")
    );
    assert!(arm.estimate.is_none());
}

#[tokio::test]
async fn a_prefix_below_the_minimum_is_disabled() {
    let mut fixture = Fixture::new();
    fixture.minimum = FakeMinimum { tokens: 40 };
    let arm = fixture.predict(BODY, &counts(LARGE_TOTAL, 39)).await;
    assert_eq!(arm.cache_state, CacheState::Disabled);
    assert_eq!(arm.reason.as_deref(), Some("below_cache_minimum"));
    assert_eq!(arm.model.as_deref(), Some(MODEL));
    assert!(arm.evidence.is_none());
    assert_eq!(
        arm.token_count_source,
        Some(TokenCountSource::AnthropicCountTokens)
    );
    let estimate = arm.estimate.clone().expect("an estimate");
    assert_eq!(estimate.tokens.uncached_input_tokens, LARGE_TOTAL);
    assert_eq!(estimate.tokens.total_tokens(), LARGE_TOTAL);
    assert_eq!(arm.cold, Some(estimate.clone()));
    assert_eq!(arm.warm, Some(estimate));
    let calls = fixture.cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tokens.uncached_input_tokens, LARGE_TOTAL);
}

#[tokio::test]
async fn a_prefix_below_the_minimum_without_a_price_is_unpriced() {
    let mut fixture = Fixture::new();
    fixture.minimum = FakeMinimum { tokens: 40 };
    fixture.cost = FakeCost::new(None);
    let arm = fixture.predict(BODY, &counts(LARGE_TOTAL, 39)).await;
    assert_eq!(arm.cache_state, CacheState::Unknown);
    assert_eq!(arm.reason.as_deref(), Some("pricing_unavailable"));
    assert!(arm.estimate.is_none());
}

#[tokio::test]
async fn an_unpriced_scenario_set_is_refused() {
    let mut fixture = Fixture::new();
    fixture.cost = FakeCost::new(None);
    let arm = fixture
        .predict(BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(arm.cache_state, CacheState::Unknown);
    assert_eq!(arm.reason.as_deref(), Some("pricing_unavailable"));
    assert!(arm.estimate.is_none());
    assert_eq!(fixture.cost.calls().len(), 3);
}

#[tokio::test]
async fn a_one_hour_prefix_creates_one_hour_tokens() {
    let fixture = Fixture::new();
    let arm = fixture
        .predict(ONE_HOUR_BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    let estimate = arm.estimate.expect("an estimate");
    assert_eq!(estimate.tokens.cache_creation_1h_input_tokens, SMALL_PREFIX);
    assert_eq!(estimate.tokens.cache_creation_5m_input_tokens, 0);
}

#[tokio::test]
async fn the_minimum_reads_the_deployment_model_name() {
    let litellm_params = params(PARAMS);
    let info = model_info();
    let deployment = PredictionDeployment::read(&litellm_params, &info);
    let resolution = Resolution::new();
    let context = resolution.context();
    let settings = settings("https://api.anthropic.com");
    let cache = MemoryCache::new();
    let cost = FakeCost::new(Some(Decimal::ONE));
    let minimum = RecordingMinimum::new(0);
    let services = prediction_services(&settings, &context, &cache, &cost, &minimum);
    let routing = AnthropicCacheRouting::from_body(&json(BODY)).expect("a predictable body");
    let counter = ScriptedCounter::new(&counts(LARGE_TOTAL, SMALL_PREFIX));
    let request = ArmRequest {
        deployment,
        caller_key_hash: CALLER,
        checked_at: CHECKED_AT,
    };
    routing.predict(&request, &counter, &services).await;
    assert_eq!(minimum.asked(), vec!["anthropic/claude-x".to_owned()]);
}

#[tokio::test]
async fn a_deployment_that_cannot_resolve_reports_the_reason() {
    let mut fixture = Fixture::new();
    fixture.litellm_params =
        params(r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","organization":"org"}"#);
    let arm = fixture
        .predict(BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(
        arm.reason.as_deref(),
        Some("unsupported_deployment_configuration")
    );
    assert_eq!(arm.cache_state, CacheState::Unknown);
    assert!(arm.estimate.is_none());
    assert!(fixture.cost.calls().is_empty());
}

#[tokio::test]
async fn a_blocked_deployment_wins_over_an_unresolvable_one() {
    let mut fixture = Fixture::new();
    fixture.litellm_params = params(
        r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_base":"https://proxy.test"}"#,
    );
    fixture.info = params(r#"{"id":"deployment-a","blocked":true}"#);
    let arm = fixture
        .predict(BODY, &counts(LARGE_TOTAL, SMALL_PREFIX))
        .await;
    assert_eq!(
        arm.reason.as_deref(),
        Some("unsupported_deployment_configuration")
    );
    assert_eq!(arm.model.as_deref(), Some("anthropic/claude-x"));
    assert!(arm.estimate.is_none());
    assert!(fixture.cost.calls().is_empty());
}

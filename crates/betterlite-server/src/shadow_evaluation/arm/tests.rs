#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::shadow_evidence::cost::PriceMap;
use betterlite_db::shadow_tables::rows::{AttemptOutcome, ShadowAttemptRow};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{ArmCalls as _, ArmFailure, AttemptDeps, RouterCalls, run_shadow_arm};
use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::error::JudgeDispatchError;
use crate::judge_targets::router::JudgeRouter;
use crate::judge_targets::sdk::JudgeSdk;
use crate::shadow_calls::{
    JudgeDeps, JudgeRequest, NO_ROUTER_ERROR, PositionCoin, ShadowDeps, ShadowRequest,
    ShadowRouter, ShadowRouterProvider,
};
use crate::shadow_eval_job_loading::jobs::{ActiveShadowEvalJob, ShadowEvalDirection};
use crate::shadow_evaluation::tests::{
    Answer, Attempts, CallsFake, SampleFixture, SpendLog, call_failure, decimal, eval_job, json,
    judge_verdict, shadow_reply,
};

const ARM: &str = "arm-router";
const CATALOG: &str = r#"{"gpt-4o": {"litellm_provider": "openai"}}"#;

fn spend_key(job_id: &str) -> String {
    format!("spend:shadow_eval:{job_id}")
}

fn reverse(job: &mut ActiveShadowEvalJob) {
    job.direction = ShadowEvalDirection::Reverse;
    job.baseline_model = Some("baseline-model".to_owned());
}

#[tokio::test]
async fn a_successful_arm_records_the_verdict_and_bills_every_leg() {
    let job = eval_job("job-1", &[ARM], None);
    let mut fixture = SampleFixture::new();
    fixture
        .shadow_params
        .insert("temperature".to_owned(), json("0.2"));
    fixture.parent_metadata.insert(
        "user_api_key_team_id".to_owned(),
        Json::Text("team-1".to_owned()),
    );
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply())],
        vec![Answer::Reply(judge_verdict("shadow"))],
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    assert_shadow_row(&attempts, &counter);
    assert_arm_calls(&calls, &fixture);
}

fn assert_shadow_row(attempts: &Attempts, counter: &SpendLog) {
    let rows = attempts.rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_shadow_identity(row);
    assert_shadow_judge_costs(row);
    assert_shadow_real_costs(row);
    assert_eq!(row.error, None);
    assert_eq!(
        counter.entries(),
        vec![(spend_key("job-1"), decimal("1.75"))]
    );
}

fn assert_shadow_identity(row: &ShadowAttemptRow) {
    assert_eq!(row.job_id, "job-1");
    assert_eq!(row.request_id, "req-1");
    assert_eq!(row.router_name, ARM);
    assert_eq!(row.outcome, AttemptOutcome::Shadow);
    assert_eq!(row.tier.as_deref(), Some("premium"));
    assert_eq!(row.real_model.as_deref(), Some("gpt-4o"));
    assert_eq!(row.shadow_model.as_deref(), Some("shadow-model"));
    assert_eq!(row.confidence, Some(decimal("0.9")));
}

fn assert_shadow_judge_costs(row: &ShadowAttemptRow) {
    assert_eq!(row.judge_cost, Decimal::ONE);
    assert_eq!(row.shadow_cost, decimal("0.25"));
    assert_eq!(row.shadow_classifier_cost, decimal("0.5"));
}

fn assert_shadow_real_costs(row: &ShadowAttemptRow) {
    assert_eq!(row.real_cost, Some(decimal("0.1")));
    assert_eq!(row.real_classifier_cost, decimal("0.02"));
    assert!(row.real_cache_hit);
}

fn assert_arm_calls(calls: &CallsFake, fixture: &SampleFixture) {
    let shadow_calls = calls.shadow_calls();
    assert_eq!(shadow_calls.len(), 1);
    assert_eq!(shadow_calls[0].target_model, ARM);
    assert_eq!(shadow_calls[0].messages, fixture.messages);
    assert_eq!(shadow_calls[0].shadow_params, fixture.shadow_params);
    assert_eq!(shadow_calls[0].parent_metadata, fixture.parent_metadata);
    let judge_calls = calls.judge_calls();
    assert_eq!(judge_calls.len(), 1);
    assert_eq!(judge_calls[0].judge_model, "judge-model");
    assert_eq!(judge_calls[0].real_text, "real answer");
    assert_eq!(judge_calls[0].shadow_text, "shadow answer");
    assert_eq!(judge_calls[0].tools, Json::Null);
    assert_eq!(judge_calls[0].messages, fixture.messages);
    assert_eq!(judge_calls[0].parent_metadata, fixture.parent_metadata);
}

#[tokio::test]
async fn the_judge_receives_the_sampled_tools_and_the_job_judge_model() {
    let mut job = eval_job("job-1", &[ARM], None);
    job.judge_model = "custom-judge".to_owned();
    let mut fixture = SampleFixture::new();
    let tools = json(r#"[{"type":"function","function":{"name":"lookup"}}]"#);
    fixture
        .shadow_params
        .insert("tools".to_owned(), tools.clone());
    fixture
        .shadow_params
        .insert("temperature".to_owned(), json("0.2"));
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply())],
        vec![Answer::Reply(judge_verdict("shadow"))],
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    assert_eq!(calls.judge_calls()[0].judge_model, "custom-judge");
    assert_eq!(calls.judge_calls()[0].tools, tools);
    assert_eq!(
        calls.shadow_calls()[0].shadow_params.get("temperature"),
        Some(&json("0.2"))
    );
}

#[tokio::test]
async fn a_reverse_job_calls_its_baseline_and_takes_the_control_tier() {
    let mut job = eval_job("job-2", &[ARM], None);
    reverse(&mut job);
    let mut fixture = SampleFixture::new();
    fixture.control_tier = Some("control".to_owned());
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply())],
        vec![Answer::Reply(judge_verdict("real"))],
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    assert_eq!(calls.shadow_targets(), vec!["baseline-model".to_owned()]);
    let rows = attempts.rows();
    assert_eq!(rows[0].outcome, AttemptOutcome::Real);
    assert_eq!(rows[0].tier.as_deref(), Some("control"));
}

#[tokio::test]
async fn a_tie_verdict_is_stored_as_a_tie() {
    let job = eval_job("job-1", &[ARM], None);
    let fixture = SampleFixture::new();
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply())],
        vec![Answer::Reply(judge_verdict("tie"))],
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    assert_eq!(attempts.rows()[0].outcome, AttemptOutcome::Tie);
}

#[tokio::test]
async fn a_preference_outside_the_three_names_is_stored_as_a_tie() {
    let job = eval_job("job-1", &[ARM], None);
    let fixture = SampleFixture::new();
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply())],
        vec![Answer::Reply(judge_verdict("A"))],
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    assert_eq!(attempts.rows()[0].outcome, AttemptOutcome::Tie);
}

#[tokio::test]
async fn a_failed_shadow_call_records_its_costs_and_skips_the_judge() {
    let job = eval_job("job-1", &[ARM], None);
    let fixture = SampleFixture::new();
    let calls = CallsFake::new(
        vec![Answer::Failed(call_failure(
            "shadow router call failed: timeout",
            "0.3",
            "0.05",
        ))],
        Vec::new(),
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    let rows = attempts.rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.outcome, AttemptOutcome::Error);
    assert_eq!(
        row.error.as_deref(),
        Some("shadow router call failed: timeout")
    );
    assert_eq!(row.shadow_cost, decimal("0.3"));
    assert_eq!(row.shadow_classifier_cost, decimal("0.05"));
    assert_eq!(row.shadow_model, None);
    assert_eq!(row.real_model, None);
    assert_eq!(row.tier, None);
    assert_eq!(row.judge_cost, Decimal::ZERO);
    assert_eq!(row.real_cost, Some(decimal("0.1")));
    assert!(calls.judge_calls().is_empty());
    assert_eq!(
        counter.entries(),
        vec![(spend_key("job-1"), decimal("0.35"))]
    );
}

#[tokio::test]
async fn a_raising_shadow_call_records_a_pipeline_error() {
    let job = eval_job("job-1", &[ARM], None);
    let fixture = SampleFixture::new();
    let calls = CallsFake::new(vec![Answer::Raised("provider exploded")], Vec::new());
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    let rows = attempts.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, AttemptOutcome::Error);
    assert_eq!(
        rows[0].error.as_deref(),
        Some("pipeline error: provider exploded")
    );
    assert_eq!(rows[0].shadow_model, None);
    assert_eq!(rows[0].shadow_cost, Decimal::ZERO);
    assert!(calls.judge_calls().is_empty());
    assert!(counter.entries().is_empty());
}

#[tokio::test]
async fn a_failed_judge_call_keeps_the_billed_shadow_and_records_the_error() {
    let job = eval_job("job-1", &[ARM], None);
    let fixture = SampleFixture::new();
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply())],
        vec![Answer::Failed(call_failure(
            "unparsable judge verdict: confidence: field required",
            "0.4",
            "0.02",
        ))],
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    let rows = attempts.rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.outcome, AttemptOutcome::Error);
    assert_eq!(
        row.error.as_deref(),
        Some("unparsable judge verdict: confidence: field required")
    );
    assert_eq!(row.shadow_model.as_deref(), Some("shadow-model"));
    assert_eq!(row.shadow_cost, decimal("0.25"));
    assert_eq!(row.shadow_classifier_cost, decimal("0.5"));
    assert_eq!(row.judge_cost, decimal("0.4"));
    assert_eq!(row.confidence, None);
    assert_eq!(row.real_model, None);
    assert_eq!(row.tier.as_deref(), Some("premium"));
    assert_eq!(
        counter.entries(),
        vec![(spend_key("job-1"), decimal("1.15"))]
    );
}

#[tokio::test]
async fn a_raising_judge_call_keeps_the_billed_shadow_and_records_a_pipeline_error() {
    let job = eval_job("job-1", &[ARM], None);
    let fixture = SampleFixture::new();
    let calls = CallsFake::new(
        vec![Answer::Reply(shadow_reply())],
        vec![Answer::Raised("judge exploded")],
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    let rows = attempts.rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.outcome, AttemptOutcome::Error);
    assert_eq!(row.error.as_deref(), Some("pipeline error: judge exploded"));
    assert_eq!(row.shadow_model.as_deref(), Some("shadow-model"));
    assert_eq!(row.shadow_cost, decimal("0.25"));
    assert_eq!(row.judge_cost, Decimal::ZERO);
    assert_eq!(row.real_model, None);
    assert_eq!(
        counter.entries(),
        vec![(spend_key("job-1"), decimal("0.75"))]
    );
}

#[tokio::test]
async fn a_reverse_shadow_failure_keeps_the_control_tier() {
    let mut job = eval_job("job-2", &[ARM], None);
    reverse(&mut job);
    let mut fixture = SampleFixture::new();
    fixture.control_tier = Some("control".to_owned());
    let calls = CallsFake::new(
        vec![Answer::Failed(call_failure(
            "shadow router call failed: timeout",
            "0.3",
            "0.05",
        ))],
        Vec::new(),
    );
    let attempts = Attempts::new();
    let counter = SpendLog::new();
    let deps = AttemptDeps {
        database: Some(&attempts),
        counter: &counter,
        calls: &calls,
    };
    run_shadow_arm(&job, ARM, &fixture.sample(), &deps).await;
    assert_eq!(attempts.rows()[0].tier.as_deref(), Some("control"));
}

struct Harness {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Harness {
    fn new() -> Self {
        Self {
            catalog: ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

struct NoRouter;

impl ShadowRouterProvider for NoRouter {
    fn shadow_router(&self) -> Option<Arc<dyn ShadowRouter>> {
        None
    }

    fn judge_router(&self) -> Option<Arc<dyn JudgeRouter>> {
        None
    }
}

struct NoPrices;

impl PriceMap for NoPrices {
    fn completion_cost(&self, _response: &Json) -> Option<Decimal> {
        None
    }
}

struct NoCoin;

impl PositionCoin for NoCoin {
    fn real_is_a(&self) -> bool {
        false
    }
}

struct NoSdk;

#[async_trait]
impl JudgeSdk for NoSdk {
    async fn acompletion(&self, _call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        Err(JudgeDispatchError {
            message: "the judge sdk is unavailable".to_owned(),
        })
    }
}

#[tokio::test]
async fn router_calls_maps_a_pod_without_a_router_to_a_failure() {
    let harness = Harness::new();
    let context = harness.context();
    let cache = ProviderCache::default();
    let provider = NoRouter;
    let prices = NoPrices;
    let coin = NoCoin;
    let sdk = NoSdk;
    let calls = RouterCalls {
        shadow: ShadowDeps {
            provider: &provider,
            prices: &prices,
        },
        judge: JudgeDeps {
            provider: &provider,
            coin: &coin,
            sdk: &sdk,
            cache: &cache,
            context: &context,
            prices: &prices,
        },
    };
    let messages = vec![Json::Text("hi".to_owned())];
    let shadow_params = IndexMap::new();
    let parent_metadata = IndexMap::new();
    assert_calls_report_failures(&calls, &messages, &shadow_params, &parent_metadata).await;
}

async fn assert_calls_report_failures(
    calls: &RouterCalls<'_>,
    messages: &[Json],
    shadow_params: &IndexMap<String, Json>,
    parent_metadata: &IndexMap<String, Json>,
) {
    let shadow_request = ShadowRequest {
        target_model: ARM,
        messages,
        shadow_params,
        parent_metadata,
    };
    let Err(ArmFailure::Failed(shadow_failure)) = calls.shadow(&shadow_request).await else {
        panic!("a pod without a router reports a failure");
    };
    assert_eq!(shadow_failure.error, NO_ROUTER_ERROR);
    assert_eq!(shadow_failure.cost, Decimal::ZERO);
    let no_tools = Json::Null;
    let judge_request = JudgeRequest {
        judge_model: "judge-model",
        messages,
        real_text: "real answer",
        shadow_text: "shadow answer",
        tools: &no_tools,
        parent_metadata,
    };
    let Err(ArmFailure::Failed(judge_failure)) = calls.judge(&judge_request).await else {
        panic!("a missing judge target reports a failure");
    };
    assert!(judge_failure.error.starts_with("judge call failed:"));
}

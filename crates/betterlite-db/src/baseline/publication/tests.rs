#![cfg(test)]

use std::future::{Future, ready};
use std::slice::from_ref;

use betterlite_models::baseline_history::estimate::{BaselineEstimate, Provenance};
use betterlite_models::baseline_history::model::{BaselineHistory, BaselineObservation, Outcome};
use betterlite_models::baseline_history::replay::advance_baseline_history;
use betterlite_models::baseline_history::usage::{
    CacheCreationTokenDetails, PromptTokensDetails, Usage,
};
use betterlite_models::decisions::Extra;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_evidence::pricing_basis::PricingBasis;
use betterlite_models::savings_evidence::snapshot::BaselineCostSnapshot;
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, TimeZone as _, Utc};
use deadpool_postgres::{Client, Pool};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::change::{Change, change};
use super::decode::decode_baseline_publication;
use super::model::{BaselinePublication, PublicationProvenance, PublicationStatus, VERSION};
use super::pricing::{PublicationPricing, baseline_publication};
use super::project::{StoredRecord, project_group};
use super::publish::{PublishError, publish};
use super::sql::{UPDATE_LOGS, UPDATE_PUBLICATIONS, session_correction_sql};
use crate::autorouter_turn_sql::tests::database;
use crate::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, TurnFlag, TurnTime};
use crate::baseline::records::canonical::serialize_record;
use crate::baseline::records::decode::{BaselineJson, BaselineJsonError, decode_baseline_json};
use crate::baseline::records::{
    BaselineAccountingRecord, BaselineEntity, DailyBaselineAttribution, DailyBaselineTarget,
};

const SCOPE: &str =
    "autorouter-baseline:v3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const MODEL: &str = "claude-sonnet-4-20250514";

struct NoPricing;

static NO_PRICING: NoPricing = NoPricing;

impl PriceLookup for NoPricing {
    fn effective_model_info(&self, _model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        None
    }
}

impl SavingsPricingContext for NoPricing {
    fn resolve_model(&self, _model: &str, _provider: Option<&str>) -> Option<ModelIdentity> {
        None
    }

    // These tests price from the record snapshot, so no deployment answers for any id.
    fn deployment_price(
        &self,
        _deployment_id: Option<&str>,
        _model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send {
        ready(None)
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        self
    }
}

fn pricing() -> PublicationPricing<'static, NoPricing> {
    PublicationPricing {
        context: &NO_PRICING,
        database: TimeZoneDatabase::from_env(),
        moment: DateTime::UNIX_EPOCH,
    }
}

fn usage() -> Usage {
    Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: Some(5),
            cached_tokens: Some(2),
            cache_creation_tokens: Some(3),
            cache_creation_token_details: Some(CacheCreationTokenDetails {
                ephemeral_5m_input_tokens: Some(1),
                ephemeral_1h_input_tokens: Some(2),
            }),
            cache_write_tokens: Some(3),
            extra: Extra::default(),
        }),
        cache_read_input_tokens: Some(2),
        cache_creation_input_tokens: Some(3),
        extra: Extra::default(),
    }
}

fn observation(request_id: &str, started_at: f64) -> BaselineObservation {
    BaselineObservation {
        request_id: request_id.to_owned(),
        started_at,
        available_at: started_at,
        outcome: Outcome::Complete,
        baseline_equivalent: true,
        usage: Some(usage()),
        plan: None,
        minimum_cache_tokens: 0,
        reason: None,
    }
}

fn incomplete_observation(request_id: &str, started_at: f64) -> BaselineObservation {
    let mut value = observation(request_id, started_at);
    value.usage = None;
    value
}

fn snapshot(actual_spend: Decimal, classifier_cost: Decimal) -> BaselineCostSnapshot {
    BaselineCostSnapshot {
        model: MODEL.to_owned(),
        provider: "anthropic".to_owned(),
        prices: None,
        basis: PricingBasis::default(),
        actual_spend,
        actual_token_cost: None,
        classifier_cost,
    }
}

fn turn(user_id: &str) -> AutoRouterTurnTransaction {
    AutoRouterTurnTransaction {
        api_key: "key".to_owned(),
        session_id: "session".to_owned(),
        router_name: "router".to_owned(),
        router_type: "simple-shuffle".to_owned(),
        model: "model".to_owned(),
        turn_at: TurnTime::Aware(
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .unwrap()
                .fixed_offset(),
        ),
        total_tokens: 15,
        spend: Decimal::new(25, 2),
        saved_spend: Decimal::ZERO,
        classifier_cost: Decimal::new(5, 2),
        covered: TurnFlag(false),
        cache_hit: TurnFlag(false),
        cache_ttl_seconds: None,
        cache_touched: TurnFlag(false),
        tier: None,
        baseline_model: Some(MODEL.to_owned()),
        savings_estimated_turns: 0,
        savings_estimated_actual_spend: Decimal::ZERO,
        savings_estimated_saved_spend: Decimal::ZERO,
        user_id: user_id.to_owned(),
    }
}

fn daily() -> DailyBaselineAttribution {
    DailyBaselineAttribution {
        date: "2026-01-02".to_owned(),
        api_key: "key".to_owned(),
        model: Some("model".to_owned()),
        custom_llm_provider: Some("anthropic".to_owned()),
        model_group: None,
        endpoint: Some("/v1/chat/completions".to_owned()),
        mcp_namespaced_tool_name: None,
        targets: vec![
            DailyBaselineTarget {
                entity: BaselineEntity::User,
                entity_id: Some("user-1".to_owned()),
            },
            DailyBaselineTarget {
                entity: BaselineEntity::Tag,
                entity_id: Some("tag-a".to_owned()),
            },
        ],
    }
}

fn accounting_record(
    base: BaselineObservation,
    turn: Option<AutoRouterTurnTransaction>,
    daily: Option<DailyBaselineAttribution>,
) -> BaselineAccountingRecord {
    BaselineAccountingRecord {
        scope: SCOPE.to_owned(),
        api_key: "key".to_owned(),
        session_id: "session".to_owned(),
        router_name: "router".to_owned(),
        baseline_model: MODEL.to_owned(),
        observation: base,
        pricing: snapshot(Decimal::new(20, 2), Decimal::new(5, 2)),
        turn,
        daily,
    }
}

fn record() -> BaselineAccountingRecord {
    accounting_record(
        observation("req-1", 2.0),
        Some(turn("user-1")),
        Some(daily()),
    )
}

fn publication(
    record: &BaselineAccountingRecord,
    estimate: &BaselineEstimate,
    first_at: f64,
) -> BaselinePublication {
    baseline_publication(&pricing(), record, estimate, first_at)
}

fn identical_estimate(record: &BaselineAccountingRecord) -> BaselineEstimate {
    BaselineEstimate {
        request_id: record.observation.request_id.clone(),
        reason: "identical_baseline_path".to_owned(),
        provenance: Some(Provenance::ObservedIdentical),
        usage: record.observation.usage.clone(),
    }
}

fn unknown_publication(reason: &str) -> BaselinePublication {
    BaselinePublication {
        version: VERSION,
        comparison_id: SCOPE.to_owned(),
        comparison_started_at: 1.0,
        status: PublicationStatus::Unknown,
        reason: reason.to_owned(),
        provenance: None,
        actual_spend: None,
        baseline_spend: None,
        input_tokens: None,
        cache_read_input_tokens: None,
        cache_creation_5m_input_tokens: None,
        cache_creation_1h_input_tokens: None,
    }
}

fn estimated_publication(actual: Decimal, baseline: Decimal) -> BaselinePublication {
    BaselinePublication {
        version: VERSION,
        comparison_id: SCOPE.to_owned(),
        comparison_started_at: 1.0,
        status: PublicationStatus::Estimated,
        reason: "identical_baseline_path".to_owned(),
        provenance: Some(PublicationProvenance::ObservedIdentical),
        actual_spend: Some(actual),
        baseline_spend: Some(baseline),
        input_tokens: Some(5),
        cache_read_input_tokens: Some(2),
        cache_creation_5m_input_tokens: Some(1),
        cache_creation_1h_input_tokens: Some(2),
    }
}

#[derive(Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct StoredPublication {
    version: i64,
    comparison_id: String,
    comparison_started_at: f64,
    status: String,
    reason: String,
    provenance: Option<String>,
    #[serde(rename = "actual_spend")]
    actual_amount: Option<f64>,
    #[serde(rename = "baseline_spend")]
    baseline_amount: Option<f64>,
    input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    cache_creation_5m_input_tokens: Option<i64>,
    cache_creation_1h_input_tokens: Option<i64>,
}

#[derive(Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct StoredTarget {
    entity: String,
    entity_id: Option<String>,
}

#[derive(Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct StoredDaily {
    date: String,
    api_key: String,
    model: Option<String>,
    custom_llm_provider: Option<String>,
    model_group: Option<String>,
    endpoint: Option<String>,
    mcp_namespaced_tool_name: Option<String>,
    targets: Vec<StoredTarget>,
}

#[derive(Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct StoredChange {
    request_id: String,
    publication: StoredPublication,
    api_key: String,
    user_id: String,
    session_id: String,
    router_name: String,
    baseline_model: String,
    covered_delta: i64,
    actual_delta: f64,
    savings_delta: f64,
    daily: Option<StoredDaily>,
}

fn decoded_change(value: &Change) -> StoredChange {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

#[test]
fn publication_serializes_every_stored_field_and_keeps_amounts_as_floats() {
    let value = estimated_publication(Decimal::new(25, 2), Decimal::new(20, 2));
    let stored: StoredPublication =
        serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
    assert_eq!(stored.version, 3);
    assert_eq!(stored.comparison_id, SCOPE);
    assert_eq!(stored.status, "estimated");
    assert_eq!(stored.provenance.as_deref(), Some("observed_identical"));
    assert_eq!(
        stored.actual_amount.map(f64::to_bits),
        Some(0.25_f64.to_bits())
    );
    assert_eq!(
        stored.baseline_amount.map(f64::to_bits),
        Some(0.20_f64.to_bits())
    );
    assert_eq!(stored.input_tokens, Some(5));
    let unknown = unknown_publication("conflicting_observation");
    let stored_unknown: StoredPublication =
        serde_json::from_str(&serde_json::to_string(&unknown).unwrap()).unwrap();
    assert_eq!(stored_unknown.provenance, None);
    assert_eq!(stored_unknown.actual_amount, None);
}

#[test]
fn costs_need_estimated_status_and_both_amounts_present() {
    let value = estimated_publication(Decimal::new(25, 2), Decimal::new(20, 2));
    let costs = value.costs().unwrap();
    assert_eq!(costs.actual, Decimal::new(25, 2));
    assert_eq!(costs.baseline, Decimal::new(20, 2));
    let mut missing = value.clone();
    missing.baseline_spend = None;
    assert_eq!(missing.costs(), None);
    let mut retracted = value;
    retracted.status = PublicationStatus::Unknown;
    assert_eq!(retracted.costs(), None);
}

#[test]
fn publication_without_pricing_keeps_usage_evidence_and_names_the_reason() {
    let record = record();
    let estimate = BaselineEstimate {
        request_id: record.observation.request_id.clone(),
        reason: "cache_prefix_available".to_owned(),
        provenance: Some(Provenance::Modeled),
        usage: Some(usage()),
    };
    let value = publication(&record, &estimate, 2.0);
    assert_eq!(value.status, PublicationStatus::Unknown);
    assert_eq!(value.reason, "pricing_unavailable");
    assert_eq!(value.provenance, None);
    assert_eq!(value.actual_spend, None);
    assert_eq!(value.baseline_spend, None);
    assert_eq!(value.input_tokens, Some(5));
    assert_eq!(value.cache_read_input_tokens, Some(2));
    assert_eq!(value.cache_creation_5m_input_tokens, Some(1));
    assert_eq!(value.cache_creation_1h_input_tokens, Some(2));
}

#[test]
fn publication_without_usage_keeps_the_history_reason_and_stays_unknown() {
    let record = accounting_record(incomplete_observation("req-1", 2.0), None, None);
    let estimate = BaselineEstimate {
        request_id: record.observation.request_id.clone(),
        reason: "concurrent_uncertainty".to_owned(),
        provenance: None,
        usage: None,
    };
    let value = publication(&record, &estimate, 2.0);
    assert_eq!(value.status, PublicationStatus::Unknown);
    assert_eq!(value.reason, "concurrent_uncertainty");
    assert_eq!(value.input_tokens, None);
}

#[test]
fn publication_prices_an_observed_identical_baseline_including_classifier_cost() {
    let record = record();
    let estimate = identical_estimate(&record);
    let value = publication(&record, &estimate, 2.0);
    assert_eq!(value.status, PublicationStatus::Estimated);
    assert_eq!(value.reason, "identical_baseline_path");
    assert_eq!(value.comparison_started_at.to_bits(), 2.0_f64.to_bits());
    assert_eq!(value.actual_spend, Some(Decimal::new(25, 2)));
    assert_eq!(value.baseline_spend, Some(Decimal::new(20, 2)));
}

#[test]
fn change_retracts_published_savings_and_actual_spend() {
    let record = record();
    let old = estimated_publication(Decimal::new(25, 2), Decimal::new(20, 2));
    let new = unknown_publication("conflicting_observation");
    let value = change(&record, Some(&old), &new);
    assert_eq!(value.request_id, "req-1");
    assert_eq!(value.user_id, "user-1");
    assert_eq!(value.covered_delta, -1);
    assert_eq!(value.actual_delta, Decimal::new(-25, 2));
    assert_eq!(value.savings_delta, Decimal::new(5, 2));
    assert!(value.daily.as_ref() == Some(&daily()));
}

#[test]
fn change_from_unknown_publishes_the_full_signed_amounts() {
    let record = record();
    let old = unknown_publication("conflicting_observation");
    let new = estimated_publication(Decimal::new(25, 2), Decimal::new(20, 2));
    let value = change(&record, Some(&old), &new);
    assert_eq!(value.covered_delta, 1);
    assert_eq!(value.actual_delta, Decimal::new(25, 2));
    assert_eq!(value.savings_delta, Decimal::new(-5, 2));
    let unchanged = change(&record, Some(&new), &new);
    assert_eq!(unchanged.covered_delta, 0);
    assert_eq!(unchanged.actual_delta, Decimal::ZERO);
    assert_eq!(unchanged.savings_delta, Decimal::ZERO);
}

#[test]
fn change_subtracts_the_correction_amounts_as_exact_decimals() {
    let record = record();
    let previous = estimated_publication(Decimal::new(1, 1), Decimal::new(3, 1));
    let current = estimated_publication(Decimal::new(4, 1), Decimal::new(7, 1));
    let value = decoded_change(&change(&record, Some(&previous), &current));
    assert_eq!(value.actual_delta.to_bits(), 0.3_f64.to_bits());
    assert_eq!(value.savings_delta.to_bits(), 0.1_f64.to_bits());
}

#[test]
fn change_serializes_the_daily_attribution_the_correction_key_reads() {
    let record = record();
    let published = estimated_publication(Decimal::new(25, 2), Decimal::new(20, 2));
    let stored = decoded_change(&change(&record, None, &published));
    assert_eq!(stored.request_id, "req-1");
    assert_eq!(stored.publication.version, 3);
    assert_eq!(stored.actual_delta.to_bits(), 0.25_f64.to_bits());
    assert_eq!(stored.savings_delta.to_bits(), (-0.05_f64).to_bits());
    let target = stored.daily.unwrap().targets;
    assert_eq!(target.len(), 2);
    assert_eq!(target[0].entity, "user");
    assert_eq!(target[0].entity_id.as_deref(), Some("user-1"));
    assert_eq!(target[1].entity, "tag");
    let retracted_record = accounting_record(incomplete_observation("req-2", 2.0), None, None);
    let retraction = change(
        &retracted_record,
        None,
        &unknown_publication("concurrent_uncertainty"),
    );
    let retracted = decoded_change(&retraction);
    assert!(retracted.daily.is_none());
    assert_eq!(retracted.user_id, "");
}

#[test]
fn decodes_publications_and_rejects_other_versions_keys_or_amounts() {
    let text = serde_json::to_string(&estimated_publication(
        Decimal::new(25, 2),
        Decimal::new(20, 2),
    ))
    .unwrap();
    let decoded = decode_baseline_publication(text.as_bytes()).unwrap();
    assert_eq!(decoded.status, PublicationStatus::Estimated);
    let other_version = text.replace("\"version\":3", "\"version\":4");
    decode_baseline_publication(other_version.as_bytes()).unwrap_err();
    let extra_key = text.replace("\"reason\"", "\"unknown\":1,\"reason\"");
    decode_baseline_publication(extra_key.as_bytes()).unwrap_err();
    let string_amount = text.replace("\"actual_spend\":0.25", "\"actual_spend\":\"0.25\"");
    decode_baseline_publication(string_amount.as_bytes()).unwrap_err();
    let huge_amount = text.replace(
        "\"baseline_spend\":0.2",
        "\"baseline_spend\":79228162514264337593543950336",
    );
    decode_baseline_publication(huge_amount.as_bytes()).unwrap_err();
    let missing_version = text.replace("\"version\":3,", "");
    decode_baseline_publication(missing_version.as_bytes()).unwrap();
}

#[test]
fn project_group_emits_a_change_only_when_the_publication_moved() {
    let record = record();
    let (history, estimates) =
        advance_baseline_history(&BaselineHistory::default(), from_ref(&record.observation))
            .unwrap();
    let expected = publication(&record, &estimates[0], 2.0);
    let stored = StoredRecord {
        data: serialize_record(&record),
        publication: Some(serde_json::to_string(&expected).unwrap()),
        conflicted: false,
        started_at: 2.0,
    };
    let (advanced, changes) = project_group(
        &pricing(),
        &(BaselineHistory::default(), Vec::new()),
        &[stored],
    )
    .unwrap();
    assert_eq!(advanced, history);
    assert!(changes.is_empty());
}

#[test]
fn project_group_appends_the_change_for_a_first_projection() {
    let record = record();
    let stored = StoredRecord {
        data: serialize_record(&record),
        publication: None,
        conflicted: false,
        started_at: 2.0,
    };
    let (_, changes) = project_group(
        &pricing(),
        &(BaselineHistory::default(), Vec::new()),
        &[stored],
    )
    .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].request_id, "req-1");
    assert_eq!(changes[0].covered_delta, 1);
    assert_eq!(changes[0].savings_delta, Decimal::new(-5, 2));
    assert_eq!(changes[0].publication.status, PublicationStatus::Estimated);
}

#[test]
fn project_group_replaces_a_conflicted_observation_with_an_uncertain_one() {
    let record = record();
    let stored = StoredRecord {
        data: serialize_record(&record),
        publication: None,
        conflicted: true,
        started_at: 2.0,
    };
    let (advanced, changes) = project_group(
        &pricing(),
        &(BaselineHistory::default(), Vec::new()),
        &[stored],
    )
    .unwrap();
    assert!(!advanced.equivalent);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].publication.status, PublicationStatus::Unknown);
    assert_eq!(changes[0].publication.reason, "conflicting_observation");
    assert_eq!(changes[0].publication.provenance, None);
}

#[test]
fn project_group_keeps_prior_changes_and_the_first_group_timestamp() {
    let record = record();
    let prior_record = accounting_record(observation("prior", 2.0), None, None);
    let prior = change(&prior_record, None, &unknown_publication("prior"));
    let previous_history = BaselineHistory {
        first_at: Some(1.5_f64),
        last_at: Some(1.0_f64),
        equivalent: true,
        uncertain_before: 0.0,
        entries: Vec::new(),
        blocked_until: 0.0,
    };
    let stored = StoredRecord {
        data: serialize_record(&record),
        publication: None,
        conflicted: false,
        started_at: 2.0,
    };
    let (_, changes) = project_group(
        &pricing(),
        &(previous_history, vec![prior.clone()]),
        &[stored],
    )
    .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].request_id, prior.request_id);
    assert_eq!(
        changes[1].publication.comparison_started_at.to_bits(),
        1.5_f64.to_bits()
    );
}

#[test]
fn decode_baseline_json_dispatches_on_the_marker_byte() {
    let record = record();
    let mut record_text = b"R".to_vec();
    record_text.extend_from_slice(serialize_record(&record).as_bytes());
    match decode_baseline_json(&record_text).unwrap() {
        BaselineJson::Record(value) => assert_eq!(value.observation.request_id, "req-1"),
        BaselineJson::Publication(_) | BaselineJson::History(_) => panic!("record marker"),
    }
    let publication_text = serde_json::to_string(&estimated_publication(
        Decimal::new(25, 2),
        Decimal::new(20, 2),
    ))
    .unwrap();
    let mut marked = b"P".to_vec();
    marked.extend_from_slice(publication_text.as_bytes());
    match decode_baseline_json(&marked).unwrap() {
        BaselineJson::Publication(value) => assert_eq!(value.status, PublicationStatus::Estimated),
        BaselineJson::Record(_) | BaselineJson::History(_) => panic!("publication marker"),
    }
    let history_text = serde_json::to_string(&BaselineHistory::default()).unwrap();
    let mut marked_history = b"H".to_vec();
    marked_history.extend_from_slice(history_text.as_bytes());
    match decode_baseline_json(&marked_history).unwrap() {
        BaselineJson::History(value) => assert_eq!(value.first_at, None),
        BaselineJson::Record(_) | BaselineJson::Publication(_) => panic!("history marker"),
    }
    assert!(matches!(
        decode_baseline_json(b"X{}").err(),
        Some(BaselineJsonError::UnknownKind)
    ));
    assert!(matches!(
        decode_baseline_json(b"").err(),
        Some(BaselineJsonError::UnknownKind)
    ));
}

#[test]
fn history_decoder_keeps_the_source_dataclass_defaults() {
    let BaselineJson::History(decoded) = decode_baseline_json(b"H{}").unwrap() else {
        panic!("history marker");
    };
    assert_eq!(*decoded, BaselineHistory::default());
}

#[test]
fn session_correction_sql_matches_the_source_statements() {
    let key = session_correction_sql(false);
    assert!(key.contains("\"LiteLLM_AutoRouterSession\" AS session"));
    assert!(!key.contains("user_id <> ''"));
    assert!(key.contains("session.api_key = totals.api_key"));
    let user = session_correction_sql(true);
    assert!(user.contains("\"LiteLLM_AutoRouterUserSession\" AS session"));
    assert!(user.contains("WHERE user_id <> ''"));
    assert!(user.contains("session.user_id = totals.user_id AND"));
    assert!(key.contains("savings_estimated_baseline_models = ("));
    assert!(key.contains("'{}'::jsonb"));
    assert!(key.contains("HAVING SUM(value::int) > 0"));
    assert!(key.contains("session.saved_spend + totals.savings_delta"));
}

#[test]
fn stored_statements_replace_the_observation_and_write_the_signed_savings() {
    assert!(UPDATE_LOGS.contains("- 'autorouter_baseline_observation'"));
    assert!(UPDATE_LOGS.contains("'autorouter_savings_estimate', changes.publication"));
    assert!(UPDATE_LOGS.contains("(changes.publication->>'baseline_spend')::float8"));
    assert!(UPDATE_PUBLICATIONS.contains("SET publication = x.publication::text"));
}

async fn seed_log(client: &Client, request_id: &str) {
    client
        .execute(
            "INSERT INTO \"LiteLLM_SpendLogs\" (request_id, call_type, api_key, \"startTime\", \"endTime\", metadata) VALUES ($1, 'acompletion', 'key', NOW(), NOW(), $2::jsonb)",
            &[
                &request_id,
                &r#"{"autorouter_baseline_observation":{"request_id":"seed"},"keep":"value"}"#,
            ],
        )
        .await
        .unwrap();
}

async fn seed_observation(client: &Client, record: &BaselineAccountingRecord) {
    client
        .execute(
            "INSERT INTO \"LiteLLM_AutoRouterBaselineObservation\" (request_id, scope, started_at, revision, data) VALUES ($1, $2, $3, 1, $4)",
            &[
                &record.observation.request_id,
                &record.scope,
                &record.observation.started_at,
                &serialize_record(record),
            ],
        )
        .await
        .unwrap();
}

async fn seed_sessions(client: &Client) {
    client
        .execute(
            "INSERT INTO \"LiteLLM_AutoRouterSession\" (api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model, saved_spend, savings_estimated_turns, savings_estimated_actual_spend, savings_estimated_saved_spend, savings_estimated_baseline_models) VALUES ('key', 'session', 'router', 'simple-shuffle', NOW(), NOW(), 'model', 1.0, 2, 0.5, 0.25, '{\"claude-sonnet-4-20250514\":2,\"retired\":1}'::jsonb)",
            &[],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO \"LiteLLM_AutoRouterUserSession\" (user_id, api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model, saved_spend, savings_estimated_turns, savings_estimated_actual_spend, savings_estimated_saved_spend, savings_estimated_baseline_models) VALUES ('user-1', 'key', 'session', 'router', 'simple-shuffle', NOW(), NOW(), 'model', 2.0, 5, 1.0, 0.5, '{\"claude-sonnet-4-20250514\":4,\"retired\":2}'::jsonb)",
            &[],
        )
        .await
        .unwrap();
}

fn retraction_change(record: &BaselineAccountingRecord) -> Change {
    let old = estimated_publication(Decimal::new(10, 2), Decimal::new(30, 2));
    let new = unknown_publication("conflicting_observation");
    change(record, Some(&old), &new)
}

fn correction_changes(record: &BaselineAccountingRecord) -> Vec<Change> {
    let published = publication(record, &identical_estimate(record), 2.0);
    let retracted = accounting_record(incomplete_observation("req-2", 2.0), None, None);
    vec![
        change(record, None, &published),
        retraction_change(&retracted),
    ]
}

async fn text_value(client: &Client, sql: &str, request_id: &str) -> Option<String> {
    let row = client.query_one(sql, &[&request_id]).await.unwrap();
    row.get::<_, Option<String>>(0)
}

async fn assert_spend_log(client: &Client, request_id: &str, expected: Option<&str>) {
    let removed = text_value(
        client,
        "SELECT (metadata ? 'autorouter_baseline_observation')::text FROM \"LiteLLM_SpendLogs\" WHERE request_id = $1",
        request_id,
    )
    .await;
    assert_eq!(removed.as_deref(), Some("false"));
    let saved = text_value(
        client,
        "SELECT metadata->>'autorouter_savings' FROM \"LiteLLM_SpendLogs\" WHERE request_id = $1",
        request_id,
    )
    .await;
    assert_eq!(saved.as_deref(), expected);
    let kept = text_value(
        client,
        "SELECT metadata->>'keep' FROM \"LiteLLM_SpendLogs\" WHERE request_id = $1",
        request_id,
    )
    .await;
    assert_eq!(kept.as_deref(), Some("value"));
}

async fn session_row(client: &Client, table: &str) -> tokio_postgres::Row {
    let statement = format!(
        "SELECT saved_spend::text, savings_estimated_turns, savings_estimated_actual_spend::text, savings_estimated_saved_spend::text, savings_estimated_baseline_models::text FROM \"{table}\""
    );
    client.query_one(&statement, &[]).await.unwrap()
}

async fn assert_sessions(client: &Client) {
    let key = session_row(client, "LiteLLM_AutoRouterSession").await;
    assert_eq!(key.get::<_, String>(0), "0.75");
    assert_eq!(key.get::<_, i32>(1), 2_i32);
    assert_eq!(key.get::<_, String>(2), "0.65");
    assert_eq!(key.get::<_, String>(3), "0");
    assert_eq!(key.get::<_, String>(4), "{\"claude-sonnet-4-20250514\": 3}");
    let user = session_row(client, "LiteLLM_AutoRouterUserSession").await;
    assert_eq!(user.get::<_, String>(0), "1.95");
    assert_eq!(user.get::<_, i32>(1), 6_i32);
    assert_eq!(user.get::<_, String>(2), "1.25");
    assert_eq!(user.get::<_, String>(3), "0.45");
    assert_eq!(
        user.get::<_, String>(4),
        "{\"claude-sonnet-4-20250514\": 5, \"retired\": 2}"
    );
}

async fn assert_daily_savings(client: &Client) {
    let row = client
        .query_one(
            "SELECT autorouter_savings_spend::text, spend::text, api_requests, request_id FROM \"LiteLLM_DailyTagSpend\"",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "-0.05");
    assert_eq!(row.get::<_, String>(1), "0");
    assert_eq!(row.get::<_, i64>(2), 0);
    assert_eq!(row.get::<_, Option<String>>(3).as_deref(), Some("req-1"));
    let user_row = client
        .query_one(
            "SELECT autorouter_savings_spend::text, user_id FROM \"LiteLLM_DailyUserSpend\"",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(user_row.get::<_, String>(0), "-0.05");
    assert_eq!(user_row.get::<_, String>(1), "user-1");
}

async fn assert_stored_publications(client: &Client) {
    let expected = publication(&record(), &identical_estimate(&record()), 2.0);
    let stored = text_value(
        client,
        "SELECT publication FROM \"LiteLLM_AutoRouterBaselineObservation\" WHERE request_id = $1",
        "req-1",
    )
    .await
    .unwrap();
    assert_eq!(
        decode_baseline_publication(stored.as_bytes()).unwrap(),
        expected
    );
    let retracted = text_value(
        client,
        "SELECT publication FROM \"LiteLLM_AutoRouterBaselineObservation\" WHERE request_id = $1",
        "req-2",
    )
    .await
    .unwrap();
    let decoded = decode_baseline_publication(retracted.as_bytes()).unwrap();
    assert_eq!(decoded.status, PublicationStatus::Unknown);
}

async fn seeded_publication_surfaces(record: &BaselineAccountingRecord) -> (PostgresFixture, Pool) {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    seed_log(&client, "req-1").await;
    seed_log(&client, "req-2").await;
    seed_sessions(&client).await;
    seed_observation(&client, record).await;
    let retracted = accounting_record(incomplete_observation("req-2", 2.0), None, None);
    seed_observation(&client, &retracted).await;
    drop(client);
    (fixture, pool)
}

#[tokio::test]
async fn postgres_publish_updates_logs_sessions_daily_rows_and_publications() {
    let record = record();
    let (fixture, pool) = seeded_publication_surfaces(&record).await;
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    publish(
        &transaction,
        &correction_changes(&record),
        uuid::Uuid::new_v4,
    )
    .await
    .unwrap();
    transaction.commit().await.unwrap();
    assert_spend_log(&client, "req-1", Some("-0.05")).await;
    assert_spend_log(&client, "req-2", None).await;
    assert_sessions(&client).await;
    assert_daily_savings(&client).await;
    assert_stored_publications(&client).await;
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_publish_skips_the_user_session_statement_for_empty_user_ids() {
    let record = accounting_record(observation("req-1", 2.0), None, None);
    let (fixture, pool) = seeded_publication_surfaces(&record).await;
    let mut client = pool.get().await.unwrap();
    let transaction = client.transaction().await.unwrap();
    let published = publication(&record, &identical_estimate(&record), 2.0);
    publish(
        &transaction,
        &[change(&record, None, &published)],
        uuid::Uuid::new_v4,
    )
    .await
    .unwrap();
    transaction.commit().await.unwrap();
    let row = client
        .query_one(
            "SELECT saved_spend::text FROM \"LiteLLM_AutoRouterUserSession\"",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "2");
    let daily = client
        .query_one("SELECT COUNT(*) FROM \"LiteLLM_DailyTagSpend\"", &[])
        .await
        .unwrap();
    assert_eq!(daily.get::<_, i64>(0), 0);
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn seed_failure_trigger(client: &Client) {
    client
        .batch_execute(
            "CREATE OR REPLACE FUNCTION baseline_publication_failure() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'injected publication failure'; END; $$ LANGUAGE plpgsql; CREATE TRIGGER baseline_publication_failure BEFORE UPDATE ON \"LiteLLM_AutoRouterBaselineObservation\" FOR EACH ROW EXECUTE FUNCTION baseline_publication_failure()",
        )
        .await
        .unwrap();
}

async fn assert_surfaces_untouched(client: &Client) {
    let metadata = text_value(
        client,
        "SELECT metadata::text FROM \"LiteLLM_SpendLogs\" WHERE request_id = $1",
        "req-1",
    )
    .await
    .unwrap();
    assert_eq!(
        metadata,
        "{\"autorouter_baseline_observation\": {\"request_id\": \"seed\"}, \"keep\": \"value\"}"
    );
    let session = client
        .query_one(
            "SELECT saved_spend::text, savings_estimated_turns FROM \"LiteLLM_AutoRouterSession\"",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(session.get::<_, String>(0), "1");
    assert_eq!(session.get::<_, i32>(1), 2_i32);
    let publication = text_value(
        client,
        "SELECT publication FROM \"LiteLLM_AutoRouterBaselineObservation\" WHERE request_id = $1",
        "req-1",
    )
    .await;
    assert_eq!(publication, None);
    let daily = client
        .query_one("SELECT COUNT(*) FROM \"LiteLLM_DailyTagSpend\"", &[])
        .await
        .unwrap();
    assert_eq!(daily.get::<_, i64>(0), 0);
}

#[tokio::test]
async fn postgres_publish_rolls_back_every_surface_when_a_later_statement_fails() {
    let record = record();
    let (fixture, pool) = seeded_publication_surfaces(&record).await;
    let mut client = pool.get().await.unwrap();
    seed_failure_trigger(&client).await;
    let transaction = client.transaction().await.unwrap();
    let published = publication(&record, &identical_estimate(&record), 2.0);
    let result = publish(
        &transaction,
        &[change(&record, None, &published)],
        uuid::Uuid::new_v4,
    )
    .await;
    assert!(matches!(result, Err(PublishError::Statement(_))));
    drop(transaction);
    assert_surfaces_untouched(&client).await;
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn postgres_publish_issues_no_statement_for_an_empty_change_list() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.unwrap();
    seed_log(&client, "req-1").await;
    publish(&client, &[], uuid::Uuid::new_v4).await.unwrap();
    let metadata = text_value(
        &client,
        "SELECT metadata::text FROM \"LiteLLM_SpendLogs\" WHERE request_id = $1",
        "req-1",
    )
    .await;
    assert_eq!(
        metadata.as_deref(),
        Some(
            "{\"autorouter_baseline_observation\": {\"request_id\": \"seed\"}, \"keep\": \"value\"}"
        )
    );
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}

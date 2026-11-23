#![cfg(test)]

use std::future::{Future, ready};

use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::DateTime;
use deadpool_postgres::{Client, Pool};
use jiff::tz::TimeZoneDatabase;
use uuid::Uuid;

use crate::autorouter_turn_sql::tests::database;
use crate::baseline::publication::decode::decode_baseline_publication;
use crate::baseline::publication::model::BaselinePublication;
use crate::baseline::publication::pricing::PublicationPricing;
use crate::baseline::records::{BaselineAccountingRecord, decode_baseline_record};
use crate::baseline::store::BaselineAccountingStore;
use crate::database_replica::pinned::WriterPinnedClient;
use crate::postgres_store::session::PostgresStore;

pub(crate) const SCOPE: &str =
    "autorouter-baseline:v3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub(crate) const OTHER_SCOPE: &str =
    "autorouter-baseline:v3:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

const LOG_ROW: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime", metadata) VALUES ($1, 'acompletion', 'key', NOW(), NOW(), '{"keep": "value"}'::jsonb)"#;
const COMPARISON: &str = r#"SELECT revision, published_revision, attempted_at IS NULL, retired, initial_equivalent, history FROM "LiteLLM_AutoRouterBaselineComparison" WHERE scope = $1"#;
const OBSERVATION: &str = r#"SELECT revision, conflicted, publication, scope FROM "LiteLLM_AutoRouterBaselineObservation" WHERE request_id = $1"#;
const SESSION: &str = r#"SELECT turns::bigint, saved_spend::text, savings_estimated_turns::bigint FROM "LiteLLM_AutoRouterSession""#;

// The records price from their own snapshot, so no deployment or public price answers.
pub(crate) struct NoPricing;

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

pub(crate) struct Harness {
    pub(crate) fixture: PostgresFixture,
    pub(crate) pool: Pool,
    pub(crate) writer: PostgresStore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Equivalence {
    Equivalent,
    Different,
}

impl From<bool> for Equivalence {
    fn from(equivalent: bool) -> Self {
        if equivalent {
            Self::Equivalent
        } else {
            Self::Different
        }
    }
}

pub(crate) struct ComparisonRow {
    pub(crate) revision: i64,
    pub(crate) published: i64,
    pub(crate) unattempted: bool,
    pub(crate) retired: bool,
    pub(crate) initial_equivalent: Equivalence,
    pub(crate) history: Option<String>,
}

pub(crate) struct ObservationRow {
    pub(crate) revision: i64,
    pub(crate) conflicted: bool,
    pub(crate) publication: Option<BaselinePublication>,
    pub(crate) scope: String,
}

pub(crate) async fn harness() -> Harness {
    let (fixture, pool) = database().await;
    Harness {
        writer: PostgresStore {
            pool: Some(pool.clone()),
        },
        fixture,
        pool,
    }
}

pub(crate) fn store(
    writer: &PostgresStore,
) -> BaselineAccountingStore<'_, NoPricing, fn() -> Uuid> {
    BaselineAccountingStore {
        database: WriterPinnedClient::Writer(writer),
        pricing: PublicationPricing {
            context: &NoPricing,
            database: TimeZoneDatabase::from_env(),
            moment: DateTime::UNIX_EPOCH,
        },
        generate_id: Uuid::new_v4,
    }
}

pub(crate) fn record(request_id: &str, started_at: f64) -> BaselineAccountingRecord {
    let mut value = decode_baseline_record(include_bytes!("fixtures/record.json")).unwrap();
    value.observation.request_id = request_id.to_owned();
    value.observation.started_at = started_at;
    value.observation.available_at = started_at;
    value
}

impl Harness {
    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}

pub(crate) async fn seed_logs(client: &Client, request_ids: &[&str]) {
    for request_id in request_ids {
        client.execute(LOG_ROW, &[request_id]).await.unwrap();
    }
}

pub(crate) async fn comparison(client: &Client, scope: &str) -> ComparisonRow {
    let row = client.query_one(COMPARISON, &[&scope]).await.unwrap();
    ComparisonRow {
        revision: row.get(0),
        published: row.get(1),
        unattempted: row.get(2),
        retired: row.get(3),
        initial_equivalent: Equivalence::from(row.get(4)),
        history: row.get(5),
    }
}

pub(crate) async fn observation(client: &Client, request_id: &str) -> Option<ObservationRow> {
    let row = client
        .query_opt(OBSERVATION, &[&request_id])
        .await
        .unwrap()?;
    let publication: Option<String> = row.get(2);
    Some(ObservationRow {
        revision: row.get(0),
        conflicted: row.get(1),
        publication: publication.map(|text| decode_baseline_publication(text.as_bytes()).unwrap()),
        scope: row.get(3),
    })
}

pub(crate) async fn reason(client: &Client, request_id: &str) -> String {
    observation(client, request_id)
        .await
        .unwrap()
        .publication
        .unwrap()
        .reason
}

// turns, saved_spend and savings_estimated_turns of the one key session the fixture records write
pub(crate) async fn session(client: &Client) -> (i64, String, i64) {
    let row = client.query_one(SESSION, &[]).await.unwrap();
    (row.get(0), row.get(1), row.get(2))
}

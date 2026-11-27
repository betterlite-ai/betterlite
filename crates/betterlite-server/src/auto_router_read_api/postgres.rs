use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::auxiliary_repository::session::AutoRouterSessionRepository;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::managed_resource_rows::autorouter::AutoRouterSession;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redis::config_sync::publish::{PublishChange, redis_config_change_publisher};
use chrono::NaiveDateTime;
use tokio_postgres::types::ToSql;

use crate::app::spend::cli_writer_store;
use crate::benchmark_aggregates::decode::decode_session_agg_rows;
use crate::benchmark_aggregates::row::SessionAggRow;
use crate::benchmark_aggregates::window::BenchmarkWindow;
use crate::state::ServerState;

use super::error::ReadApiError;
use super::store::AutoRouterReadStore;

// AUTOROUTER_BENCHMARKS_SQL, kept verbatim and wrapped so the rows come back as one jsonb array
pub(crate) const BENCHMARKS_SQL: &str = r##"
WITH windowed AS (
    SELECT api_key, session_id, router_name, router_type, first_turn_at, last_turn_at,
        last_model, models, turns, unordered_turns, covered_turns, cache_hits,
        same_model_turns, same_model_hits, first_visit_turns, first_visit_hits,
        return_turns, return_hits, return_expired_misses, return_within_ttl_misses,
        ttl_5m_turns, ttl_1h_turns, total_tokens, spend, saved_spend, classifier_cost,
        classifier_cost_recorded_turns, tier_turns, baseline_models, savings_estimated_turns,
        savings_estimated_actual_spend, savings_estimated_saved_spend,
        savings_estimated_baseline_models, NULL::text AS comparison_user_id
    FROM "LiteLLM_AutoRouterSession"
    WHERE $4::text IS NULL
      AND last_turn_at >= $1::timestamp
      AND first_turn_at < $2::timestamp
      AND ($3::text IS NULL OR api_key = $3::text)
    UNION ALL
    SELECT api_key, session_id, router_name, router_type, first_turn_at, last_turn_at,
        last_model, models, turns, unordered_turns, covered_turns, cache_hits,
        same_model_turns, same_model_hits, first_visit_turns, first_visit_hits,
        return_turns, return_hits, return_expired_misses, return_within_ttl_misses,
        ttl_5m_turns, ttl_1h_turns, total_tokens, spend, saved_spend, classifier_cost,
        classifier_cost_recorded_turns, tier_turns, baseline_models, savings_estimated_turns,
        savings_estimated_actual_spend, savings_estimated_saved_spend,
        savings_estimated_baseline_models, user_id AS comparison_user_id
    FROM "LiteLLM_AutoRouterUserSession"
    WHERE (($4::text IS NOT NULL AND user_id = $4::text) OR ($4::text IS NULL AND api_key = ''))
      AND last_turn_at >= $1::timestamp
      AND first_turn_at < $2::timestamp
      AND ($3::text IS NULL OR api_key = $3::text)
),
tier_maps AS (
    SELECT router_name, router_type, jsonb_object_agg(tier, tier_turns) AS tier_turns
    FROM (
        SELECT router_name, router_type, kv.key AS tier, SUM((kv.value)::int)::int AS tier_turns
        FROM windowed, LATERAL jsonb_each_text(tier_turns) AS kv
        GROUP BY router_name, router_type, kv.key
    ) per_tier
    GROUP BY router_name, router_type
)
SELECT COALESCE(jsonb_agg(to_jsonb(agg_rows) ORDER BY agg_rows.spend DESC), '[]'::jsonb)::text
FROM (
    SELECT agg.*, COALESCE(tier_maps.tier_turns, '{}'::jsonb) AS tier_turns
    FROM (
        SELECT
            router_name,
            router_type,
            COUNT(*)::int AS sessions,
            COALESCE(SUM(turns), 0)::int AS turns,
            COALESCE(SUM(unordered_turns), 0)::int AS unordered_turns,
            COALESCE(SUM(covered_turns), 0)::int AS covered_turns,
            COALESCE(SUM(cache_hits), 0)::int AS cache_hits,
            COALESCE(SUM(same_model_turns), 0)::int AS same_model_turns,
            COALESCE(SUM(same_model_hits), 0)::int AS same_model_hits,
            COALESCE(SUM(first_visit_turns), 0)::int AS first_visit_turns,
            COALESCE(SUM(first_visit_hits), 0)::int AS first_visit_hits,
            COALESCE(SUM(return_turns), 0)::int AS return_turns,
            COALESCE(SUM(return_hits), 0)::int AS return_hits,
            COALESCE(SUM(return_expired_misses), 0)::int AS return_expired_misses,
            COALESCE(SUM(return_within_ttl_misses), 0)::int AS return_within_ttl_misses,
            COALESCE(SUM(ttl_5m_turns), 0)::int AS ttl_5m_turns,
            COALESCE(SUM(ttl_1h_turns), 0)::int AS ttl_1h_turns,
            COALESCE(SUM(total_tokens), 0)::bigint AS total_tokens,
            COALESCE(SUM(spend), 0)::float8 AS spend,
            COALESCE(SUM(saved_spend), 0)::float8 AS saved_spend,
            COALESCE(SUM(savings_estimated_turns), 0)::int AS savings_estimated_turns,
            COALESCE(SUM(savings_estimated_actual_spend), 0)::float8 AS savings_estimated_actual_spend,
            CASE WHEN BOOL_AND(savings_estimated_turns = turns AND classifier_cost_recorded_turns = turns)
                THEN SUM(classifier_cost)::float8 END AS savings_estimated_classifier_cost,
            COALESCE(SUM(savings_estimated_saved_spend), 0)::float8 AS savings_estimated_saved_spend,
            COALESCE(SUM(classifier_cost), 0)::float8 AS classifier_cost,
            COALESCE(SUM(classifier_cost_recorded_turns), 0)::int AS classifier_cost_recorded_turns,
            COALESCE(SUM(EXTRACT(EPOCH FROM (last_turn_at - first_turn_at))), 0)::float8 AS session_seconds
        FROM windowed
        GROUP BY router_name, router_type
    ) agg
    LEFT JOIN tier_maps USING (router_name, router_type)
) AS agg_rows
"##;

pub(crate) struct PgAutoRouterReads {
    pub(crate) store: PostgresStore,
    pub(crate) availability: Availability,
}

#[async_trait]
impl AutoRouterReadStore for PgAutoRouterReads {
    async fn agg_rows(
        &self,
        window: &BenchmarkWindow,
        api_key: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<SessionAggRow>, ReadApiError> {
        let client = self.store.session().await?;
        let start = window_bound(&window.query_start)?;
        let end = window_bound(&window.query_end)?;
        let params: [&(dyn ToSql + Sync); 4] = [&start, &end, &api_key, &user_id];
        let row = client
            .query_one(BENCHMARKS_SQL, &params)
            .await
            .map_err(|source| ReadApiError::from(backend::<Infallible>(source)))?;
        let text: String = row
            .try_get(0)
            .map_err(|source| ReadApiError::from(backend::<Infallible>(source)))?;
        decode_session_agg_rows(text.as_bytes()).map_err(ReadApiError::from)
    }

    async fn latest_session(
        &self,
        api_key: &str,
        session_id: &str,
    ) -> Result<Option<AutoRouterSession>, ReadApiError> {
        let client = PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        };
        let publish: PublishChange<TableError> = redis_config_change_publisher(None);
        AutoRouterSessionRepository::new(Some(client), publish)
            .find_latest_for_key(api_key, session_id)
            .await
            .map_err(ReadApiError::from)
    }
}

// the SQL casts its window bounds to timestamp, so they are bound as timestamps rather than text
fn window_bound(value: &str) -> Result<NaiveDateTime, ReadApiError> {
    value
        .parse::<NaiveDateTime>()
        .map_err(|source| ReadApiError::Store {
            message: source.to_string(),
        })
}

// the route reads through the proxy's writer pool, the one the cache activity routes already open
#[must_use]
pub(crate) fn reads_from_state(state: &Arc<ServerState>) -> Option<Arc<dyn AutoRouterReadStore>> {
    let store = cli_writer_store(state)?;
    Some(Arc::new(PgAutoRouterReads {
        store: PostgresStore::clone(&store),
        availability: Availability::default(),
    }))
}

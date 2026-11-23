use async_trait::async_trait;
use betterlite_db::postgres_store::errors::{DbError, backend};
use rust_decimal::Decimal;
use serde::Deserialize;

use super::store::PostgresLogResults;
use crate::spend::log_query::sql::{AGENT_CALL_TYPE_SQL, MCP_CALL_TYPES_SQL};

// _SESSION_MODELS_LIMIT, plus the extra row that only sets the truncation flag
pub const SESSION_MODELS_LIMIT: usize = 10;
const SESSION_MODELS_FETCH: i32 = 11;
// _SESSION_MODEL_NAME_MAX_LEN
const SESSION_MODEL_NAME_MAX_LEN: i32 = 256;

// _build_ui_spend_logs_response's aggregate: one GROUP BY (session_id, api_key) serves the whole page
const AGGREGATE_SQL_TEMPLATE: &str = r#"
SELECT row_to_json(x)::text
FROM (
    SELECT s.*, COALESCE(m.session_models, ARRAY[]::text[]) AS session_models
    FROM (
        SELECT session_id, api_key,
               COUNT(*)::int AS session_total_count,
               COALESCE(SUM(spend), 0)::double precision AS session_total_spend,
               COALESCE(SUM(
                   COALESCE(
                       request_duration_ms,
                       (EXTRACT(EPOCH FROM ("endTime" - "startTime")) * 1000)::INTEGER
                   )
               ), 0)::bigint AS session_total_duration_ms,
               COUNT(*) FILTER (
                   WHERE call_type IN {mcp_call_types}
               )::int AS mcp_tool_call_count,
               COALESCE(SUM(spend) FILTER (
                   WHERE call_type IN {mcp_call_types}
               ), 0)::double precision AS mcp_tool_call_spend,
               COUNT(*) FILTER (WHERE LOWER(cache_hit) = 'true')::int AS session_cache_hit_count,
               COUNT(*) FILTER (
                   WHERE call_type NOT IN {mcp_call_types} AND call_type != {agent_call_type}
               )::int AS session_llm_count,
               COUNT(*) FILTER (WHERE call_type = {agent_call_type})::int AS session_agent_count,
               COALESCE(SUM(prompt_tokens), 0)::bigint AS session_total_prompt_tokens,
               COALESCE(SUM(completion_tokens), 0)::bigint AS session_total_completion_tokens,
               COALESCE(SUM(total_tokens), 0)::bigint AS session_total_tokens
        FROM "LiteLLM_SpendLogs"
        WHERE session_id = ANY($1::text[])
          AND api_key = ANY($2::text[])
        GROUP BY session_id, api_key
    ) s
    LEFT JOIN LATERAL (
        SELECT ARRAY_AGG(d.model ORDER BY d.model) AS session_models
        FROM (
            SELECT DISTINCT LEFT(model, $3::int) AS model
            FROM "LiteLLM_SpendLogs"
            WHERE session_id = s.session_id
              AND api_key = s.api_key
              AND model IS NOT NULL AND model <> ''
            ORDER BY 1
            LIMIT $4::int
        ) d
    ) m ON TRUE
) x
"#;

// the session ids and api keys the page already shows, so two callers reusing an id never share totals
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionLookup {
    pub session_ids: Vec<String>,
    pub api_keys: Vec<String>,
}

// _SessionSpendStats, decoded from the aggregate row; no Debug since api_key holds a key or its hash
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct SessionAggregate {
    pub session_id: String,
    pub api_key: String,
    pub session_total_count: i32,
    #[serde(deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize")]
    pub session_total_spend: Decimal,
    pub session_total_duration_ms: i64,
    pub mcp_tool_call_count: i32,
    #[serde(deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize")]
    pub mcp_tool_call_spend: Decimal,
    pub session_cache_hit_count: i32,
    pub session_llm_count: i32,
    pub session_agent_count: i32,
    pub session_total_prompt_tokens: i64,
    #[serde(rename = "session_total_completion_tokens")]
    pub session_completion_tokens: i64,
    pub session_total_tokens: i64,
    pub session_models: Vec<String>,
    #[serde(skip)]
    pub session_models_truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionAggregateError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the session aggregate row could not be decoded")]
    Row(#[source] serde_json::Error),
}

#[async_trait]
pub trait SessionAggregateReader: Send + Sync {
    async fn session_aggregates(
        &self,
        lookup: &SessionLookup,
    ) -> Result<Vec<SessionAggregate>, SessionAggregateError>;
}

#[async_trait]
impl SessionAggregateReader for PostgresLogResults {
    async fn session_aggregates(
        &self,
        lookup: &SessionLookup,
    ) -> Result<Vec<SessionAggregate>, SessionAggregateError> {
        let client = self.store.session().await?;
        let sql = AGGREGATE_SQL_TEMPLATE
            .replace("{mcp_call_types}", MCP_CALL_TYPES_SQL)
            .replace("{agent_call_type}", AGENT_CALL_TYPE_SQL);
        let rows = client
            .query(
                &sql,
                &[
                    &lookup.session_ids,
                    &lookup.api_keys,
                    &SESSION_MODEL_NAME_MAX_LEN,
                    &SESSION_MODELS_FETCH,
                ],
            )
            .await
            .map_err(|source| SessionAggregateError::Database(backend(source)))?;
        let mut aggregates = Vec::with_capacity(rows.len());
        for row in &rows {
            let text: String = row
                .try_get(0)
                .map_err(|source| SessionAggregateError::Database(backend(source)))?;
            let mut aggregate: SessionAggregate =
                serde_json::from_str(&text).map_err(SessionAggregateError::Row)?;
            aggregate.session_models_truncated =
                aggregate.session_models.len() > SESSION_MODELS_LIMIT;
            aggregate.session_models.truncate(SESSION_MODELS_LIMIT);
            aggregates.push(aggregate);
        }
        Ok(aggregates)
    }
}

const API_KEY: &str = "$1";
const SESSION_ID: &str = "$2";
const ROUTER_NAME: &str = "$3";
const ROUTER_TYPE: &str = "$4";
const MODEL: &str = "$5";
const TURN_AT: &str = "$6";
const TOTAL_TOKENS: &str = "$7";
const SPEND: &str = "$8";
const SAVED_SPEND: &str = "$9";
const CLASSIFIER_COST: &str = "$10";
const COVERED: &str = "$11";
const CACHE_HIT: &str = "$12";
const CACHE_TTL: &str = "$13";
const CACHE_TOUCHED: &str = "$14";
const TIER: &str = "$15";
const BASELINE: &str = "$16";
const SAVINGS_TURNS: &str = "$17";
const SAVINGS_ACTUAL: &str = "$18";
const SAVINGS_SAVED: &str = "$19";
const USER_ID: &str = "$20";

const CACHE_TTL_5M_SECONDS: i64 = 300;
const CACHE_TTL_1H_SECONDS: i64 = 3600;

const SESSION_COLUMNS: &str = "api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, \
    last_model, models, turns, unordered_turns, covered_turns, cache_hits, \
    same_model_turns, same_model_hits, first_visit_turns, first_visit_hits, \
    return_turns, return_hits, return_expired_misses, return_within_ttl_misses, \
    ttl_5m_turns, ttl_1h_turns, total_tokens, spend, saved_spend, classifier_cost, classifier_cost_recorded_turns, tier_turns, \
    baseline_models, savings_estimated_turns, savings_estimated_actual_spend, savings_estimated_saved_spend, \
    savings_estimated_baseline_models";

const IN_ORDER: &str = "$6::timestamp >= t.last_turn_at";
const SAME: &str = "$6::timestamp >= t.last_turn_at AND t.last_model = $5";
const FIRST: &str = "$6::timestamp >= t.last_turn_at AND NOT t.models ? $5";
const RETURN: &str = "$6::timestamp >= t.last_turn_at AND t.models ? $5 AND t.last_model <> $5";
const RETURN_MISS: &str = "$6::timestamp >= t.last_turn_at AND t.models ? $5 AND t.last_model <> $5 \
    AND $11::int = 1 AND $12::int = 0 AND (t.models -> $5 ->> 'ttl') IS NOT NULL";
const IDLE_SECONDS: &str = "EXTRACT(EPOCH FROM $6::timestamp) - (t.models -> $5 ->> 'at')::float8";
fn select_values(user_scoped: bool) -> String {
    let user_value = if user_scoped {
        format!("{USER_ID}::text, ")
    } else {
        String::new()
    };
    let tier_delta = format!(
        "(CASE WHEN {TIER}::text IS NULL THEN '{{}}'::jsonb ELSE jsonb_build_object({TIER}::text, 1) END)"
    );
    let baseline_delta = format!(
        "(CASE WHEN {BASELINE}::text IS NULL THEN '{{}}'::jsonb ELSE jsonb_build_object({BASELINE}::text, 1) END)"
    );
    let estimated_baseline_delta = format!(
        "(CASE WHEN ({SAVINGS_TURNS}::int = 1 AND {BASELINE}::text IS NOT NULL) THEN jsonb_build_object({BASELINE}::text, 1) ELSE '{{}}'::jsonb END)"
    );
    format!(
        "{user_value}{API_KEY}, {SESSION_ID}, {ROUTER_NAME}, {ROUTER_TYPE}, {TURN_AT}::timestamp, {TURN_AT}::timestamp,\n    \
        {MODEL}, jsonb_build_object({MODEL}, jsonb_build_object('at', EXTRACT(EPOCH FROM {TURN_AT}::timestamp), 'ttl', {CACHE_TTL}::int)),\n    \
        1, 0, {COVERED}::int, {CACHE_HIT}::int,\n    \
        0, 0, 1, {CACHE_HIT}::int,\n    \
        0, 0, 0, 0,\n    \
        (CASE WHEN {CACHE_TTL}::int = {CACHE_TTL_5M_SECONDS} THEN 1 ELSE 0 END),\n    \
        (CASE WHEN {CACHE_TTL}::int = {CACHE_TTL_1H_SECONDS} THEN 1 ELSE 0 END),\n    \
        {TOTAL_TOKENS}::bigint, {SPEND}::float8, {SAVED_SPEND}::float8,\n    \
        {CLASSIFIER_COST}::float8, 1, {tier_delta}, {baseline_delta},\n    \
        {SAVINGS_TURNS}::int, {SAVINGS_ACTUAL}::float8,\n    \
        {SAVINGS_SAVED}::float8, {estimated_baseline_delta}"
    )
}

fn conflict_counters() -> String {
    format!(
        "turns = t.turns + 1,\n    \
        total_tokens = t.total_tokens + EXCLUDED.total_tokens,\n    \
        spend = t.spend + EXCLUDED.spend,\n    \
        saved_spend = t.saved_spend + EXCLUDED.saved_spend,\n    \
        savings_estimated_turns = t.savings_estimated_turns + EXCLUDED.savings_estimated_turns,\n    \
        savings_estimated_actual_spend = t.savings_estimated_actual_spend + EXCLUDED.savings_estimated_actual_spend,\n    \
        savings_estimated_saved_spend = t.savings_estimated_saved_spend + EXCLUDED.savings_estimated_saved_spend,\n    \
        classifier_cost = t.classifier_cost + EXCLUDED.classifier_cost,\n    \
        classifier_cost_recorded_turns = t.classifier_cost_recorded_turns + 1,\n    \
        covered_turns = t.covered_turns + EXCLUDED.covered_turns,\n    \
        cache_hits = t.cache_hits + EXCLUDED.cache_hits,\n    \
        ttl_5m_turns = t.ttl_5m_turns + EXCLUDED.ttl_5m_turns,\n    \
        ttl_1h_turns = t.ttl_1h_turns + EXCLUDED.ttl_1h_turns,\n    \
        unordered_turns = t.unordered_turns + (CASE WHEN NOT ({IN_ORDER}) THEN 1 ELSE 0 END),\n    \
        same_model_turns = t.same_model_turns + (CASE WHEN {SAME} THEN 1 ELSE 0 END),\n    \
        same_model_hits = t.same_model_hits + (CASE WHEN {SAME} AND {CACHE_HIT}::int = 1 THEN 1 ELSE 0 END),\n    \
        first_visit_turns = t.first_visit_turns + (CASE WHEN {FIRST} THEN 1 ELSE 0 END),\n    \
        first_visit_hits = t.first_visit_hits + (CASE WHEN {FIRST} AND {CACHE_HIT}::int = 1 THEN 1 ELSE 0 END),\n    \
        return_turns = t.return_turns + (CASE WHEN {RETURN} THEN 1 ELSE 0 END),\n    \
        return_hits = t.return_hits + (CASE WHEN {RETURN} AND {CACHE_HIT}::int = 1 THEN 1 ELSE 0 END),\n    \
        return_expired_misses = t.return_expired_misses\n        \
        + (CASE WHEN {RETURN_MISS} AND {IDLE_SECONDS} > (t.models -> {MODEL} ->> 'ttl')::float8 THEN 1 ELSE 0 END),\n    \
        return_within_ttl_misses = t.return_within_ttl_misses\n        \
        + (CASE WHEN {RETURN_MISS} AND {IDLE_SECONDS} <= (t.models -> {MODEL} ->> 'ttl')::float8 THEN 1 ELSE 0 END)"
    )
}

fn conflict_models_and_tiers(router_type_param: &str) -> String {
    let estimated_baseline = format!("{SAVINGS_TURNS}::int = 1 AND {BASELINE}::text IS NOT NULL");
    format!(
        "models = t.models || jsonb_build_object({MODEL}, jsonb_build_object(\n        \
        'at', (CASE WHEN {CACHE_TOUCHED}::int = 1\n               \
        THEN GREATEST(COALESCE((t.models -> {MODEL} ->> 'at')::float8, 0), EXTRACT(EPOCH FROM {TURN_AT}::timestamp))\n               \
        ELSE COALESCE((t.models -> {MODEL} ->> 'at')::float8, EXTRACT(EPOCH FROM {TURN_AT}::timestamp)) END),\n        \
        'ttl', (CASE WHEN {IN_ORDER}\n                \
        THEN COALESCE({CACHE_TTL}::int, (t.models -> {MODEL} ->> 'ttl')::int)\n                \
        ELSE COALESCE((t.models -> {MODEL} ->> 'ttl')::int, {CACHE_TTL}::int) END)\n    \
        )),\n    \
        last_model = (CASE WHEN {IN_ORDER} THEN {MODEL} ELSE t.last_model END),\n    \
        tier_turns = (CASE WHEN {TIER}::text IS NOT NULL AND t.router_type = {router_type_param}\n        \
        THEN t.tier_turns || jsonb_build_object({TIER}::text, COALESCE((t.tier_turns ->> {TIER}::text)::int, 0) + 1)\n        \
        ELSE t.tier_turns END),\n    \
        baseline_models = (CASE WHEN {BASELINE}::text IS NOT NULL\n        \
        THEN t.baseline_models || jsonb_build_object({BASELINE}::text, COALESCE((t.baseline_models ->> {BASELINE}::text)::int, 0) + 1)\n        \
        ELSE t.baseline_models END),\n    \
        savings_estimated_baseline_models = (CASE WHEN {estimated_baseline}\n        \
        THEN t.savings_estimated_baseline_models || jsonb_build_object(\n            \
        {BASELINE}::text, COALESCE((t.savings_estimated_baseline_models ->> {BASELINE}::text)::int, 0) + 1)\n        \
        ELSE t.savings_estimated_baseline_models END),\n    \
        first_turn_at = LEAST(t.first_turn_at, EXCLUDED.first_turn_at),\n    \
        last_turn_at = GREATEST(t.last_turn_at, EXCLUDED.last_turn_at)"
    )
}

#[must_use]
pub fn session_upsert_sql(user_scoped: bool) -> String {
    let table_name = if user_scoped {
        "LiteLLM_AutoRouterUserSession"
    } else {
        "LiteLLM_AutoRouterSession"
    };
    let user_column = if user_scoped { "user_id, " } else { "" };
    let required_identity = if user_scoped { USER_ID } else { API_KEY };
    format!(
        "INSERT INTO \"{table_name}\" AS t (\n    {user_column}{SESSION_COLUMNS}\n)\nSELECT\n    {}\nWHERE {required_identity}::text <> ''\nON CONFLICT ({user_column}api_key, session_id, router_name) DO UPDATE SET\n    {},\n    {}",
        select_values(user_scoped),
        conflict_counters(),
        conflict_models_and_tiers(ROUTER_TYPE),
    )
}

#[must_use]
pub fn upsert_autorouter_session_sql() -> String {
    format!(
        "WITH key_rollup AS (\n    {}\n    RETURNING 1\n)\n{}",
        session_upsert_sql(false),
        session_upsert_sql(true),
    )
}

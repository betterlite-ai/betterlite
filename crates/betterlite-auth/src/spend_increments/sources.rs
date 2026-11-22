use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;

use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::layers::CounterLayers;

use super::error::SpendIncrementError;

const BUDGET_LIMITS: &str = "budget_limits";

// the budget windows the key or team object cached under cache_key carries; a legacy JSON string is parsed and anything else names no window
pub(super) async fn cached_budget_limits<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    cache_key: &str,
) -> Result<Vec<JsonData>, SpendIncrementError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    let Some(Json::Object(fields)) = writes.sources.cached(cache_key).await else {
        return Ok(Vec::new());
    };
    let Some(limits) = fields.get(BUDGET_LIMITS) else {
        return Ok(Vec::new());
    };
    match limits {
        Json::List(items) => Ok(items.iter().map(json_data).collect()),
        Json::Text(text) => parse_budget_limits(text, cache_key),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(Vec::new()),
    }
}

// json.loads of a budget_limits string: a parse failure is the scope's error, as LiteLLM's json.loads exception is
fn parse_budget_limits(text: &str, cache_key: &str) -> Result<Vec<JsonData>, SpendIncrementError> {
    let limits: JsonData =
        serde_json::from_str(text).map_err(|source| SpendIncrementError::BudgetLimits {
            cache_key: cache_key.to_owned(),
            source,
        })?;
    match limits {
        JsonData::Array(items) => Ok(items),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Object(_) => Ok(Vec::new()),
    }
}

// the auth cache hands back its own JSON data model; the window parser reads JsonData
fn json_data(value: &Json) -> JsonData {
    match value {
        Json::Null => JsonData::Null,
        Json::Bool(flag) => JsonData::Bool(*flag),
        Json::Number(number) => JsonData::Number(number.clone()),
        Json::Text(text) => JsonData::String(text.clone()),
        Json::List(items) => JsonData::Array(items.iter().map(json_data).collect()),
        Json::Object(fields) => JsonData::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), json_data(field)))
                .collect(),
        ),
    }
}

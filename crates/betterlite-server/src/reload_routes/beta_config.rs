use std::collections::BTreeMap;

use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_db::budget_permission_rows::JsonData;

use super::error::{ReloadError, store_call};
use super::ports::ReloadConfigStore;

pub const BETA_RELOAD_CONFIG_PARAM_NAME: &str = "anthropic_beta_headers_reload_config";

const INTERVAL_HOURS_FIELD: &str = "interval_hours";
const FORCE_RELOAD_FIELD: &str = "force_reload";
const PROVIDER_ALIASES_KEY: &str = "provider_aliases";
const DESCRIPTION_KEY: &str = "description";

// {"interval_hours": hours | None, "force_reload": flag}: the row LiteLLM writes through safe_dumps
pub(super) fn reload_config_value(interval_hours: Option<i64>, force_reload: bool) -> JsonData {
    let mut fields = BTreeMap::new();
    fields.insert(
        INTERVAL_HOURS_FIELD.to_owned(),
        interval_hours.map_or(JsonData::Null, |hours| {
            JsonData::Number(serde_json::Number::from(hours))
        }),
    );
    fields.insert(FORCE_RELOAD_FIELD.to_owned(), JsonData::Bool(force_reload));
    JsonData::Object(fields)
}

pub(super) fn interval_hours(value: JsonData) -> Option<i64> {
    let JsonData::Object(fields) = value else {
        return None;
    };
    match fields.get(INTERVAL_HOURS_FIELD) {
        Some(JsonData::Number(number)) => number.as_i64(),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::String(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => None,
    }
}

// the manual reload keeps the stored interval, replaces it with null when there was no row, and forces the other pods
pub(super) async fn write_force_reload_row(
    store: &dyn ReloadConfigStore,
    prefix: &'static str,
) -> Result<(), ReloadError> {
    let existing = store_call(store.read_beta_config(), prefix).await?;
    let interval = existing.and_then(interval_hours);
    let value = reload_config_value(interval, true);
    store_call(store.write_beta_config(value), prefix).await?;
    store_call(store.invalidate_beta_config(), prefix).await
}

pub(super) fn provider_count(matrix: &BetaMatrix) -> usize {
    matrix
        .0
        .keys()
        .filter(|key| key.as_str() != PROVIDER_ALIASES_KEY && key.as_str() != DESCRIPTION_KEY)
        .count()
}

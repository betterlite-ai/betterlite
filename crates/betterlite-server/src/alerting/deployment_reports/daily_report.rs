use std::fmt::Write as _;

use betterlite_logging::latency_alerts::api_base::ApiBaseResolver;
use betterlite_logging::latency_alerts::presentation::float_text;
use betterlite_models::api_base_lookup::ApiBaseError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use serde_json::Number;

use super::deployment_metrics::{DailyReportCache, MetricKeys, top_five};

// Router.get_model_ids and Router.get_model_info, reduced to the two lookups the report makes.
pub trait DailyReportRouter: Send + Sync {
    fn model_ids(&self) -> Vec<String>;

    fn model_info(&self, id: &str) -> Option<DeploymentSpec>;
}

#[must_use]
pub fn daily_report_message(
    failed_rows: &[String],
    latency_rows: &[String],
    now_seconds: f64,
    next_run: f64,
) -> String {
    let mut message = String::new();
    let _ignored = write!(
        message,
        "\n\nTime: `{}`s\nHere are today's key metrics \u{1f4c8}: \n\n",
        float_text(now_seconds)
    );
    let _ignored = write!(
        message,
        "\n\n*\u{2757}\u{fe0f} Top Deployments with Most Failed Requests:*\n\n"
    );
    append_rows(&mut message, failed_rows);
    let _ignored = write!(message, "\n\n*\u{1f605} Top Slowest Deployments:*\n\n");
    append_rows(&mut message, latency_rows);
    let _ignored = write!(message, "\n\nNext Run is at: `{}`s", float_text(next_run));
    message
}

// The failed-request rows, or none when a top-five id has no deployment in the router.
pub fn failed_requests_section<Router>(
    api_base: &dyn ApiBaseResolver,
    router: &Router,
    keys: &[String],
    values: &[Option<f64>],
) -> Result<Option<Vec<String>>, ApiBaseError>
where
    Router: DailyReportRouter,
{
    let mut rows = Vec::new();
    for (rank, index) in top_five(values).into_iter().enumerate() {
        let Some(key) = keys.get(index) else {
            return Ok(None);
        };
        let id = key.split(':').next().unwrap_or(key);
        let Some(spec) = router.model_info(id) else {
            return Ok(None);
        };
        let name = deployment_name(&spec);
        let resolved = api_base.resolve(&name, &spec.litellm_params)?;
        let base = resolved.as_deref().unwrap_or("");
        let value = values.get(index).copied().flatten().unwrap_or_default();
        rows.push(format!(
            "\t{}. Deployment: `{name}`, Failed Requests: `{}`,  API Base: `{base}`\n",
            rank.saturating_add(1),
            count_text(value)
        ));
    }
    Ok(Some(rows))
}

// The slowest rows: an id the router does not know still reports, with an empty name, and Python's None stands in for a base.
pub fn latency_section<Router>(
    api_base: &dyn ApiBaseResolver,
    router: &Router,
    keys: &[String],
    values: &[Option<f64>],
) -> Result<Vec<String>, ApiBaseError>
where
    Router: DailyReportRouter,
{
    let mut rows = Vec::new();
    for (rank, index) in top_five(values).into_iter().enumerate() {
        let Some(key) = keys.get(index) else {
            continue;
        };
        let id = key.split(':').next().unwrap_or(key);
        let spec = router.model_info(id);
        let name = spec.as_ref().map_or_else(String::new, deployment_name);
        let empty = JsonObject::new();
        let params = spec.as_ref().map_or(&empty, |found| &found.litellm_params);
        let resolved = api_base.resolve(&name, params)?;
        let base = resolved.as_deref().unwrap_or("None");
        let value = values.get(index).copied().flatten().unwrap_or_default();
        rows.push(format!(
            "\t{}. Deployment: `{name}`, Latency per output token: `{}s/token`,  API Base: `{base}`\n\n",
            rank.saturating_add(1),
            latency_text(value)
        ));
    }
    Ok(rows)
}

// The cleanup before the send: every latency key, then every failed key, each set back to zero.
pub async fn reset_entries<Cache>(cache: &Cache, keys: &MetricKeys)
where
    Cache: DailyReportCache,
{
    let zero = Json::Number(Number::from(0_u8));
    let mut entries: Vec<(String, Json)> = keys
        .latency
        .iter()
        .map(|key| (key.clone(), zero.clone()))
        .collect();
    entries.extend(keys.failed.iter().map(|key| (key.clone(), zero.clone())));
    cache.set_many(&entries).await;
}

fn append_rows(message: &mut String, rows: &[String]) {
    if rows.is_empty() {
        message.push_str("\tNone\n");
        return;
    }
    for row in rows {
        message.push_str(row);
    }
}

fn deployment_name(spec: &DeploymentSpec) -> String {
    match spec.litellm_params.get("model") {
        Some(Json::Text(model)) => model.clone(),
        _ => String::new(),
    }
}

// Python's f-string of an int count: the cache's failed-request counter is always whole.
fn count_text(value: f64) -> String {
    if value.fract() == 0.0_f64 {
        return format!("{value:.0}");
    }
    float_text(value)
}

// round(value, 3), then Python's str of the float that comes back.
fn latency_text(value: f64) -> String {
    let formatted = format!("{value:.3}");
    let rounded = formatted.parse::<f64>().unwrap_or(value);
    float_text(rounded)
}

mod tests;

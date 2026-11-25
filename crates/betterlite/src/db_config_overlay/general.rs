use std::collections::BTreeMap;
use std::iter::once;

use betterlite_config::sections::settings::coerce_bool::coerce_bool;
use betterlite_config::sections::settings::{
    DbRow, FieldSource, JsonValue, Section, SettingsStore,
};
use betterlite_pyhost::json_values::JsonObject;
use betterlite_secret_managers::secret_lookup::text_to_bool;
use betterlite_server::plugin_routes::config::PluginRegistry;

use super::error::OverlayError;
use super::pass_through::{
    PassThroughInstaller, publish_pass_through, serve_pass_through_endpoints,
};
use super::scrub::scrub_remote_modules;
use super::state::{
    OverlayState, PASS_THROUGH_ENDPOINTS, RuntimeEffect, RuntimeSink, settings_mapping,
};
use crate::cleanup_schedule::cache_size::on_cache_size_changed;
use crate::cleanup_schedule::changes::{ScheduleChange, on_settings_changed};
use crate::cleanup_schedule::ports::{CacheMaxSize, CleanupScheduler, GeneralSettingsView as _};
use crate::cleanup_schedule::schedule::{CleanupSchedule, cleanup_schedule_of};
use crate::cleanup_schedule::trigger::ScheduleContext;
use crate::job_scheduler::JobFn;

const CACHE_MAX_SIZE: &str = "user_api_key_cache_max_size";
const STORE_MODEL_IN_DB: &str = "store_model_in_db";
const COERCED_FLAGS: [&str; 3] = [
    "store_prompts_in_spend_logs",
    "disable_auto_add_proxy_admin_to_teams",
    "apply_user_budget_to_team_keys",
];

// the alerting keys a database-owned row also sets on the running proxy, each with the effect it installs
type DbOwnedEffect = (&'static str, fn(JsonValue) -> RuntimeEffect);

// What the spend log cleanup hook needs: the live scheduler, its trigger context and cleanup_old_spend_logs as the job
pub struct CleanupInputs<'run> {
    pub scheduler: Option<&'run mut (dyn CleanupScheduler + Send)>,
    pub context: &'run ScheduleContext<'run>,
    pub job: JobFn,
}

pub struct EffectPorts<'ports> {
    pub cache: &'ports (dyn CacheMaxSize + Sync),
    pub plugins: &'ports PluginRegistry,
}

// _update_general_settings: what the cache and cleanup hooks compare against is read before the row lands
pub async fn apply_general_settings_row(
    state: &mut OverlayState,
    row: &JsonValue,
    cleanup: CleanupInputs<'_>,
    ports: &EffectPorts<'_>,
    installer: &dyn PassThroughInstaller,
) -> Result<(), OverlayError> {
    let current = &state.config.general_settings;
    let cache_size_was_db = current.resolution(CACHE_MAX_SIZE).source() == FieldSource::Db;
    let previous = cleanup_schedule_of(current);
    let values = settings_mapping(Some(&scrub_remote_modules(Section::GeneralSettings, row)));
    state
        .config
        .general_settings
        .apply_db_row(DbRow::GeneralSettings, values.clone());
    apply_alerting(
        &state.config.general_settings,
        &values,
        state.runtime.as_ref(),
        ports.plugins,
    )?;
    apply_pass_through_setting(state, &values, installer).await?;
    coerce_flags(&mut state.config.general_settings, &values)?;
    on_cache_size_changed(
        &values,
        cache_size_was_db,
        &mut state.config.general_settings,
        ports.cache,
    )?;
    apply_store_model_in_db(state, &values)?;
    apply_retention(state, cleanup, &previous);
    apply_ssrf(&values, state.runtime.as_ref());
    Ok(())
}

// _update_general_settings pass_through_endpoints: a list is served, and any other value keeps the empty list published
async fn apply_pass_through_setting(
    state: &mut OverlayState,
    values: &BTreeMap<String, JsonValue>,
    installer: &dyn PassThroughInstaller,
) -> Result<(), OverlayError> {
    if let Some(JsonValue::Array(endpoints)) = values.get(PASS_THROUGH_ENDPOINTS) {
        return serve_pass_through_endpoints(state, endpoints, installer).await;
    }
    let served = state.config.general_settings.get(PASS_THROUGH_ENDPOINTS);
    if served.is_none() {
        publish_pass_through(state, &[])?;
    }
    Ok(())
}

// _apply_retention_settings: B8371's hook decides whether the cleanup job moves
fn apply_retention(
    state: &mut OverlayState,
    cleanup: CleanupInputs<'_>,
    previous: &CleanupSchedule,
) {
    let change = ScheduleChange {
        previous,
        context: cleanup.context,
    };
    let settings = &state.config.general_settings;
    let memory = &mut state.cleanup;
    match cleanup.scheduler {
        Some(live) => on_settings_changed(Some(live), &change, settings, cleanup.job, memory),
        None => on_settings_changed(None, &change, settings, cleanup.job, memory),
    }
}

// _apply_alerting_settings: the alerting list follows any row that has it, the rest only when the database owns the key
fn apply_alerting(
    store: &SettingsStore,
    values: &BTreeMap<String, JsonValue>,
    runtime: &dyn RuntimeSink,
    plugins: &PluginRegistry,
) -> Result<(), OverlayError> {
    if values.contains_key("alerting")
        && let Some(JsonValue::Array(alerting)) = store.get("alerting")
    {
        runtime.apply(RuntimeEffect::Alerting(alerting.clone()));
    }
    let from_db =
        |key: &str| values.contains_key(key) && store.resolution(key).source() == FieldSource::Db;
    let db_owned: [DbOwnedEffect; 3] = [
        ("alerting_args", RuntimeEffect::AlertingArgs),
        ("alert_types", RuntimeEffect::AlertTypes),
        ("alert_to_webhook_url", RuntimeEffect::AlertToWebhookUrl),
    ];
    for (key, effect) in db_owned {
        if let Some(value) = store.get(key).filter(|_| from_db(key)) {
            runtime.apply(effect(value.clone()));
        }
    }
    if let Some(entries) = store.get("plugins").filter(|_| from_db("plugins")) {
        let settings: JsonObject = once(("plugins".to_owned(), entries.clone())).collect();
        plugins.replace_from_settings(&settings)?;
    }
    Ok(())
}

// _apply_boolean_settings: a flag the row sets is stored as a bool unless the config file owns it
fn coerce_flags(
    store: &mut SettingsStore,
    values: &BTreeMap<String, JsonValue>,
) -> Result<(), OverlayError> {
    for key in COERCED_FLAGS {
        if !values.contains_key(key) || store.owned_by_config(key) {
            continue;
        }
        if let Some(value) = store.get(key).filter(|value| !value.is_null()).cloned() {
            store.insert(key, coerce_bool(value))?;
        }
    }
    Ok(())
}

// _apply_store_model_in_db_setting: the running flag follows the row, and the store keeps it as a bool unless the file owns it
fn apply_store_model_in_db(
    state: &mut OverlayState,
    values: &BTreeMap<String, JsonValue>,
) -> Result<(), OverlayError> {
    let store = &mut state.config.general_settings;
    if !values.contains_key(STORE_MODEL_IN_DB) {
        return Ok(());
    }
    let Some(value) = store
        .get(STORE_MODEL_IN_DB)
        .filter(|value| !value.is_null())
        .cloned()
    else {
        return Ok(());
    };
    let enabled = coerce_bool(value) == JsonValue::Bool(true);
    state.config.store_model_in_db = enabled;
    if !store.owned_by_config(STORE_MODEL_IN_DB) {
        store.insert(STORE_MODEL_IN_DB, JsonValue::Bool(enabled))?;
    }
    Ok(())
}

// _apply_ssrf_general_settings: read from the row itself, and a text flag counts only as true or false
fn apply_ssrf(values: &BTreeMap<String, JsonValue>, runtime: &dyn RuntimeSink) {
    if let Some(hosts) = values.get("user_url_allowed_hosts") {
        runtime.apply(RuntimeEffect::UserUrlAllowedHosts(hosts.clone()));
    }
    let validation = match values.get("user_url_validation") {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::String(text)) => text_to_bool(text),
        Some(other) => Some(coerce_bool(other.clone()) == JsonValue::Bool(true)),
    };
    if let Some(enabled) = validation {
        runtime.apply(RuntimeEffect::UserUrlValidation(enabled));
    }
    if let Some(hosts) = values.get("provider_url_destination_allowed_hosts") {
        runtime.apply(RuntimeEffect::ProviderUrlAllowedHosts(hosts.clone()));
    }
}

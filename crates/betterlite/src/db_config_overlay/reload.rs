use std::collections::BTreeMap;
use std::iter::once;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::includes::ConfigMap;
use betterlite_config::sections::secret_manager_bootstrap::Raw as BootstrapRaw;
use betterlite_config::sections::settings::{DbRow, JsonValue, Section, SettingsStore};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_server::proxy_config_state::snapshot::ConfigWithBaseline;

use super::environ::{Resolver, owned_secret_lookup};
use super::error::OverlayError;
use super::pass_through::endpoints_beside_db;
use super::rows::apply_db_rows;
use super::source::ConfigSource;
use super::state::{
    OverlayDatabase, OverlayState, PASS_THROUGH_ENDPOINTS, STORE_SECTIONS, settings_mapping,
    store_mut,
};

const GENERAL_SETTINGS: &str = "general_settings";

// What the resolution needs after the sync half: the resolved config and an owned lookup to read its references with.
pub struct PendingReload {
    config: ConfigMap,
    lookup: SecretLookup,
    env: BTreeMap<String, String>,
}

// _load_yaml_settings_stores: every store takes the file values, and the serving pass-through list stays
pub fn reload_stores(
    state: &mut OverlayState,
    file_config: &ConfigMap,
) -> Result<(), OverlayError> {
    for section in STORE_SECTIONS {
        if let Some(store) = store_mut(&mut state.config, section) {
            reload_store(store, section, file_config.get(section.name()))?;
        }
    }
    Ok(())
}

// _reload_settings_store: the file values replace the old ones, the stale row goes, and the serving pass-through list stays
fn reload_store(
    store: &mut SettingsStore,
    section: Section,
    section_config: Option<&JsonValue>,
) -> Result<(), OverlayError> {
    let serving = store
        .get(PASS_THROUGH_ENDPOINTS)
        .filter(|value| !value.is_null())
        .cloned();
    store.load_yaml(settings_mapping(section_config));
    store.apply_db_row(DbRow::from(section), BTreeMap::new());
    if let Some(endpoints) = serving.filter(|_| section.is_resource_list(PASS_THROUGH_ENDPOINTS)) {
        store.insert(PASS_THROUGH_ENDPOINTS, endpoints)?;
    }
    Ok(())
}

// The sync half of get_config: the fetched rows reach the stores, the sections resolve, and the manager starts before its references do.
pub fn begin_reload(
    state: &mut OverlayState,
    file_config: &ConfigMap,
    stored: Option<(&OverlayDatabase, [Option<JsonValue>; 4])>,
    source: &ConfigSource,
    env: &mut BTreeMap<String, String>,
) -> Result<PendingReload, OverlayError> {
    if let Some((backed, rows)) = stored {
        apply_db_rows(state, rows, backed, env);
    }
    let config = resolved_config(state, file_config);
    initialize_secret_manager(state, &config, source, env)?;
    let lookup = owned_secret_lookup(&state.secrets.lookup);
    Ok(PendingReload {
        config,
        lookup,
        env: env.clone(),
    })
}

// _check_for_os_environ_vars, off the runtime since a manager read blocks
pub async fn resolve_reload(
    PendingReload {
        config,
        lookup,
        env,
    }: PendingReload,
) -> Result<ConfigMap, OverlayError> {
    Resolver {
        lookup: &lookup,
        env: &env,
    }
    .resolve_config(config)
    .await
}

// _apply_resolved_runtime_settings and update_config_state
pub fn finish_reload(state: &mut OverlayState, config: ConfigMap) -> ConfigWithBaseline {
    for section in STORE_SECTIONS {
        let Some(values) = config.get(section.name()).filter(|value| value.is_object()) else {
            continue;
        };
        if let Some(store) = store_mut(&mut state.config, section) {
            store.apply_runtime_values(settings_mapping(Some(values)));
        }
    }
    state.config.replace_snapshot(&config);
    ConfigWithBaseline::with_baseline(config.clone(), config)
}

// _config_with_resolved_settings: a section the file has as a mapping, or one its store holds values for, comes from the store
fn resolved_config(state: &OverlayState, file_config: &ConfigMap) -> ConfigMap {
    let mut config = file_config.clone();
    for section in STORE_SECTIONS {
        let Some(store) = state.config.settings_store(section) else {
            continue;
        };
        let file_section = file_config.get(section.name());
        if !file_section.is_some_and(JsonValue::is_object) && store.iter().next().is_none() {
            continue;
        }
        let mut values = store.resolved();
        let config_endpoints = file_section
            .and_then(|fields| fields.get(PASS_THROUGH_ENDPOINTS))
            .filter(|endpoints| !endpoints.is_null());
        let db_endpoints = store.db_value(PASS_THROUGH_ENDPOINTS);
        if config_endpoints.is_some()
            || db_endpoints.is_some_and(JsonValue::is_array)
            || values.contains_key(PASS_THROUGH_ENDPOINTS)
        {
            let merged = endpoints_beside_db(
                db_endpoints
                    .and_then(JsonValue::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                config_endpoints
                    .and_then(JsonValue::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            );
            values.insert(PASS_THROUGH_ENDPOINTS.to_owned(), JsonValue::Array(merged));
        }
        config.insert(
            section.name().to_owned(),
            JsonValue::Object(values.into_iter().collect()),
        );
    }
    config
}

// _initialize_secret_manager_from_raw_config: the manager comes up before the references it may hold are resolved
fn initialize_secret_manager(
    state: &mut OverlayState,
    config: &ConfigMap,
    source: &ConfigSource,
    env: &BTreeMap<String, String>,
) -> Result<(), OverlayError> {
    let Some(general) = config
        .get(GENERAL_SETTINGS)
        .filter(|value| value.is_object())
    else {
        return Ok(());
    };
    let wrapped: JsonValue = once((GENERAL_SETTINGS, general.clone())).collect();
    let raw = serde_json::from_value::<BootstrapRaw>(wrapped)
        .map_err(|error| OverlayError::SecretSettings { source: error })?;
    let projection = raw
        .convert(env)
        .map_err(|error| OverlayError::SecretProjection { source: error })?;
    state
        .secrets
        .initialize(&projection, source.config_file_path.as_deref(), env, None)?;
    Ok(())
}

use std::collections::BTreeMap;
use std::iter::once;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::includes::ConfigMap;
use betterlite_config::sections::secret_manager_bootstrap::Raw as BootstrapRaw;
use betterlite_config::sections::settings::{DbRow, JsonValue, Section, SettingsStore};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_server::proxy_config_state::snapshot::ConfigWithBaseline;

use super::environ::{
    ResolvedConfig, Resolver, Workers, off_runtime, owned_secret_lookup, restore_oidc_tokens,
};
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
    let serving = store.get(PASS_THROUGH_ENDPOINTS).cloned();
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
    Ok(PendingReload { config, lookup })
}

// _check_for_os_environ_vars, off the runtime since a manager read blocks and the lookup is already owned
pub async fn resolve_reload(
    pending: PendingReload,
    environment: BTreeMap<String, String>,
    workers: &Workers,
) -> Result<ResolvedConfig, OverlayError> {
    let PendingReload { config, lookup } = pending;
    off_runtime(workers, move || {
        let mut values = config;
        let outcome = Resolver {
            lookup: &lookup,
            env: &environment,
        }
        .resolve_entries(values.values_mut(), 0);
        outcome.map(|()| (values, lookup.oidc_cache))
    })
    .await?
}

// _apply_resolved_runtime_settings and update_config_state
pub fn finish_reload(state: &mut OverlayState, resolved: ResolvedConfig) -> ConfigWithBaseline {
    let (config, oidc_cache) = resolved;
    restore_oidc_tokens(&state.secrets.lookup, oidc_cache);
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
            let db_list = db_endpoints.and_then(JsonValue::as_array);
            let file_list = config_endpoints.and_then(JsonValue::as_array);
            let merged = endpoints_beside_db(
                db_list.map(Vec::as_slice).unwrap_or_default(),
                file_list.map(Vec::as_slice).unwrap_or_default(),
            );
            values.insert(PASS_THROUGH_ENDPOINTS.to_owned(), JsonValue::Array(merged));
        }
        let resolved = JsonValue::Object(values.into_iter().collect());
        config.insert(section.name().to_owned(), resolved);
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
    let settings = config.get(GENERAL_SETTINGS);
    let Some(general) = settings.filter(|value| value.is_object()) else {
        return Ok(());
    };
    let wrapped: JsonValue = once((GENERAL_SETTINGS, general.clone())).collect();
    let raw = serde_json::from_value::<BootstrapRaw>(wrapped)
        .map_err(|error| OverlayError::SecretSettings { source: error })?;
    let projection = raw
        .convert(env)
        .map_err(|error| OverlayError::SecretProjection { source: error })?;
    let path = source.config_file_path.as_deref();
    state.secrets.initialize(&projection, path, env, None)?;
    Ok(())
}

use betterlite_logging::analytics::python::repr;
use betterlite_models::decisions::Json;
use betterlite_server::ui_settings_state::cache::sync_ui_settings as apply_ui_settings;
use betterlite_server::ui_settings_state::projection::GeneralSettingsTarget;
use betterlite_server::ui_settings_state::repository::UiSettingsRepository;
use tracing::info;

pub async fn sync_ui_settings(
    repository: Option<&dyn UiSettingsRepository>,
    target: GeneralSettingsTarget<'_>,
) {
    let Some(repository) = repository else {
        return;
    };
    let applied = apply_ui_settings(repository, target).await;
    if applied.is_empty() {
        return;
    }
    let names: Vec<Json> = applied.keys().cloned().map(Json::Text).collect();
    let listed = repr(&Json::List(names)).unwrap_or_default();
    info!("Synced UI settings to general_settings on startup: {listed}");
}

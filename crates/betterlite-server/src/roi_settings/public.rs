use betterlite_roi::settings::DEFAULT_PROMPT;
use betterlite_roi::settings::ROISettings;
use betterlite_roi::settings::ROISettingsResponse;
use secrecy::ExposeSecret as _;

use super::deployments::RouterCatalog;
use super::deployments::router_models;

pub(super) fn public_settings(
    settings: &ROISettings,
    catalog: Option<&dyn RouterCatalog>,
) -> ROISettingsResponse {
    let available_models = router_models(catalog);
    let ready = !settings.repos.is_empty()
        && !settings.estimator_model.is_empty()
        && available_models.contains(&settings.estimator_model);
    ROISettingsResponse {
        github_api_url: settings.github_api_url.clone(),
        repos: settings.repos.clone(),
        estimator_model: settings.estimator_model.clone(),
        estimator_prompt: settings.estimator_prompt.clone(),
        backfill_days: settings.backfill_days,
        update_interval_minutes: settings.update_interval_minutes,
        has_estimator_key: !settings.estimator_key.expose_secret().is_empty(),
        identity_map: settings.identity_map.clone(),
        has_github_token: !settings.github_token.expose_secret().is_empty(),
        default_prompt: DEFAULT_PROMPT.to_owned(),
        available_models,
        ready,
    }
}

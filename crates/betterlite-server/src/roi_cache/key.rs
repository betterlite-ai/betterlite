use betterlite_roi::estimator::json::{JsonError, json_dumps_text};
use betterlite_roi::settings::ROISettings;
use sha2::{Digest as _, Sha256};

use crate::roi_github::models::GitHubPullListItem;

const KEY_VERSION: &str = "pull-v1";

// GitHubPullListItem keeps no field presence, so a missing body key reads like a null one
pub fn cache_key(
    settings: &ROISettings,
    context: &str,
    repo: &str,
    pull: &GitHubPullListItem,
) -> Result<Option<String>, JsonError> {
    let head = pull.head.as_ref().map_or("", |head| head.sha.as_str());
    let login = pull
        .user
        .as_ref()
        .and_then(|user| user.login.as_deref())
        .unwrap_or("");
    if head.is_empty() || login.is_empty() {
        return Ok(None);
    }
    let value = json_dumps_text(&(
        KEY_VERSION,
        settings.github_api_url.trim_end_matches('/'),
        context,
        caseless::default_case_fold_str(repo),
        pull.number,
        head,
        pull.title.as_str(),
        pull.body.as_deref().unwrap_or(""),
        caseless::default_case_fold_str(login),
    ))?;
    Ok(Some(hex::encode(Sha256::digest(value.as_bytes()))))
}

pub fn settings_fingerprint(settings: &ROISettings) -> Result<String, JsonError> {
    let value = json_dumps_text(&(
        settings.github_api_url.trim_end_matches('/'),
        &settings.repos,
        settings.estimator_model.as_str(),
        settings.estimator_prompt.as_str(),
        settings.backfill_days,
    ))?;
    Ok(hex::encode(Sha256::digest(value.as_bytes())))
}

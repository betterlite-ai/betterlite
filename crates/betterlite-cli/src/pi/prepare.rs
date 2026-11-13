use std::io::Write;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::FileAccess;
use secrecy::SecretString;

use super::limits::fetch_model_limits;
use super::listing::fetch_model_ids;
use super::models_json::{models_json_path, sync_models_json};
use super::{PI_PROVIDER_NAME, PiSyncError, ProxyGet};

pub struct PreparePiContext<'context> {
    pub environment: &'context dyn EnvironmentLookup,
    pub get: &'context dyn ProxyGet,
    pub files: &'context dyn FileAccess,
    pub notice: &'context mut dyn Write,
}

pub fn prepare_pi(
    base_url: &str,
    api_key: &SecretString,
    context: &mut PreparePiContext<'_>,
) -> Result<Vec<String>, PiSyncError> {
    let model_ids = match fetch_model_ids(base_url, api_key, context.get, &[]) {
        Ok(model_ids) => model_ids,
        Err(PiSyncError::NoModels) => return Err(PiSyncError::NothingToRun),
        Err(error) => return Err(error),
    };
    let limits = fetch_model_limits(base_url, api_key, context.get);
    let Some(path) = models_json_path(context.environment) else {
        return Err(PiSyncError::NoHome);
    };
    sync_models_json(&path, base_url, &model_ids, &limits, context.files)?;
    let _unused = writeln!(
        context.notice,
        "litellm: synced {} proxy models into {}",
        model_ids.len(),
        path.display()
    );
    let Some(first) = model_ids.first() else {
        return Err(PiSyncError::NothingToRun);
    };
    Ok(vec![
        "--model".to_owned(),
        format!("{PI_PROVIDER_NAME}/{first}"),
    ])
}

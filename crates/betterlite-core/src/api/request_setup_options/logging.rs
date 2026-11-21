use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_a::environment::EnvironmentLookup;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;

use crate::api::request_principal_metadata::team_callbacks::team_callbacks;
use crate::api::request_principal_metadata::{Metadata, PrincipalFacts};
use crate::api::request_setup_options::callbacks::{
    CallbackSettings, DYNAMIC_PARAM_NAMES, logging_entries, merge_callback, validated_callbacks,
};
use crate::api::request_setup_options::error::RequestSetupOptionsError;
use crate::api::request_setup_options::{
    CallbackSource, CallbackVarsDecryption, RequestSetupOptions, SetupDiagnostic,
};

const CALLBACK_SETTINGS_KEY: &str = "callback_settings";

// the deprecated team_metadata.callback_settings shape, read into TeamCallbackMetadata
#[derive(Debug, Default, Deserialize)]
struct StoredCallbackSettings {
    success_callback: Option<Vec<String>>,
    failure_callback: Option<Vec<String>>,
    callbacks: Option<Vec<String>>,
    callback_vars: Option<IndexMap<String, SecretString>>,
}

impl<Ids, Decryption, Environment> RequestSetupOptions<Ids, Decryption, Environment>
where
    Decryption: CallbackVarsDecryption,
    Environment: EnvironmentLookup,
{
    // the key's logging list, then the team's, then the team's old callback_settings, then default_team_settings
    pub fn dynamic_logging_settings(
        &self,
        principal: &PrincipalFacts,
        diagnostics: &mut Vec<SetupDiagnostic>,
    ) -> Result<Option<CallbackSettings>, RequestSetupOptionsError> {
        if let Some(entries) = logging_entries(&principal.metadata, &self.decryption) {
            return Ok(merged_callbacks(&entries, CallbackSource::Key, diagnostics));
        }
        let team_metadata = principal.team_metadata.as_ref();
        if let Some(entries) =
            team_metadata.and_then(|metadata| logging_entries(metadata, &self.decryption))
        {
            return Ok(merged_callbacks(
                &entries,
                CallbackSource::Team,
                diagnostics,
            ));
        }
        if let Some(metadata) =
            team_metadata.filter(|metadata| metadata.contains_key(CALLBACK_SETTINGS_KEY))
        {
            return stored_callback_settings(&self.decryption.decrypt(metadata)).map(Some);
        }
        let Some(team_id) = principal.team_id.as_deref() else {
            return Ok(None);
        };
        let configured = team_callbacks(
            team_id,
            &self.settings.default_team_settings,
            &self.environment,
        )
        .map_err(|source| RequestSetupOptionsError::TeamCallbacks { source })?;
        Ok(configured.map(|team| CallbackSettings {
            success_callback: team.success_callback,
            failure_callback: team.failure_callback,
            callbacks: Vec::new(),
            callback_vars: team.callback_vars,
        }))
    }
}

// no valid entry leaves the level unset, so the request keeps the proxy's own callbacks
fn merged_callbacks(
    entries: &Json,
    source: CallbackSource,
    diagnostics: &mut Vec<SetupDiagnostic>,
) -> Option<CallbackSettings> {
    let mut merged = None;
    for callback in validated_callbacks(entries, source, diagnostics) {
        merge_callback(
            merged.get_or_insert_with(CallbackSettings::default),
            &callback,
        );
    }
    merged
}

fn stored_callback_settings(
    team_metadata: &Metadata,
) -> Result<CallbackSettings, RequestSetupOptionsError> {
    let stored = match team_metadata.get(CALLBACK_SETTINGS_KEY) {
        None | Some(Json::Null) => StoredCallbackSettings::default(),
        Some(value) => serde_json::to_vec(value)
            .and_then(|bytes| serde_json::from_slice::<StoredCallbackSettings>(&bytes))
            .map_err(|source| RequestSetupOptionsError::CallbackSettings { source })?,
    };
    let callback_vars = stored.callback_vars.unwrap_or_default();
    if let Some(name) = callback_vars
        .keys()
        .find(|name| !DYNAMIC_PARAM_NAMES.contains(&name.as_str()))
    {
        return Err(RequestSetupOptionsError::CallbackSettingsVariable { name: name.clone() });
    }
    Ok(CallbackSettings {
        success_callback: stored.success_callback.unwrap_or_default(),
        failure_callback: stored.failure_callback.unwrap_or_default(),
        callbacks: stored.callbacks.unwrap_or_default(),
        callback_vars,
    })
}

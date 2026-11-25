use std::collections::BTreeMap;

use betterlite_auth::sso_mappings::repository::SsoSettingsRows;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::database_faults::reconnect::{
    DbReconnect, RetryPolicy, call_with_db_reconnect_retry,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::SettingsReloadError;
use super::SettingsReloader;
use super::settings_value::database_fault;
use crate::db_row_decrypt::env::decrypt_and_export_env;
use crate::db_row_decrypt::warnings::Warnings;

const LOOKUP_REASON: &str = "init_sso_settings_in_db_lookup_failure";

const DROPPED_KEYS: [&str; 3] = ["role_mappings", "team_mappings", "ui_access_mode"];

pub(super) async fn apply_sso<Db, Client>(
    reloader: &mut SettingsReloader<Client>,
    db: &Db,
) -> Result<(), SettingsReloadError>
where
    Db: SsoSettingsRows + ?Sized,
    Client: DbReconnect + Send,
{
    let stored = call_with_db_reconnect_retry(
        reloader.reconnect.as_mut(),
        move || async move {
            db.stored_sso_settings()
                .await
                .map_err(|error| database_fault(&error))
        },
        RetryPolicy {
            reason: LOOKUP_REASON,
            retry_safe_error_types: None,
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await?;
    let Some(text) = stored else {
        return Ok(());
    };
    let exported = export_sso_settings(&reloader.cipher, &reloader.warnings, &text)?;
    for (name, value) in exported {
        reloader.environment.overlay.insert(name, Some(value));
    }
    Ok(())
}

fn export_sso_settings(
    cipher: &DbCipher,
    warnings: &Warnings,
    text: &str,
) -> Result<BTreeMap<String, String>, SettingsReloadError> {
    let parsed: JsonValue = serde_json::from_str(text)?;
    let JsonValue::Object(mut settings) = parsed else {
        return Err(SettingsReloadError::SsoNotAnObject);
    };
    for key in DROPPED_KEYS {
        settings.remove(key);
    }
    let uppercased: JsonMap = settings
        .into_iter()
        .map(|(key, value)| (key.to_uppercase(), value))
        .collect();
    let mut target = BTreeMap::new();
    Ok(decrypt_and_export_env(
        &uppercased,
        false,
        cipher,
        &mut target,
        warnings,
    ))
}

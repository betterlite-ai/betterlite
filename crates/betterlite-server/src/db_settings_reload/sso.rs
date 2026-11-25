use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::sso_mappings::repository::SsoSettingsRows;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::database_faults::reconnect::{
    DbReconnect, RetryPolicy, call_with_db_reconnect_retry,
};
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::SettingsReloadError;
use super::SettingsReloader;
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
    let Some(text) = read_sso_row(reloader.reconnect.as_mut(), db).await? else {
        return Ok(());
    };
    let exported = export_sso_settings(&reloader.cipher, &reloader.warnings, &text)?;
    for (name, value) in exported {
        reloader.environment.overlay.insert(name, Some(value));
    }
    Ok(())
}

// Keep the typed table failure and the retry helper's database error, which carries any reconnect failure.
pub(super) async fn read_sso_row<Db, Client>(
    reconnect: Option<&mut Client>,
    db: &Db,
) -> Result<Option<String>, SettingsReloadError>
where
    Db: SsoSettingsRows + ?Sized,
    Client: DbReconnect + Send,
{
    let fault: Arc<Mutex<Option<TableError>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&fault);
    let read = call_with_db_reconnect_retry(
        reconnect,
        move || {
            let attempt = Arc::clone(&slot);
            async move {
                match db.stored_sso_settings().await {
                    Ok(stored) => Ok(stored),
                    Err(error) => {
                        let classification = database_fault(&error);
                        *attempt.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                        Err(classification)
                    }
                }
            }
        },
        RetryPolicy {
            reason: LOOKUP_REASON,
            retry_safe_error_types: None,
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await;
    match read {
        Ok(stored) => Ok(stored),
        Err(database) => {
            let recorded = fault.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some(source) = recorded {
                return Err(SettingsReloadError::SsoRow { source, database });
            }
            Err(SettingsReloadError::Database(database))
        }
    }
}

// The retry helper classifies faults by type, so a native transport error has to arrive as Connect.
fn database_fault(error: &TableError) -> DbError {
    let transport =
        matches!(error, TableError::Database(native) if is_native_transport_error(native));
    DbError {
        error_type: if transport {
            ErrorType::Connect
        } else {
            ErrorType::Other
        },
        message: error.to_string(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
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

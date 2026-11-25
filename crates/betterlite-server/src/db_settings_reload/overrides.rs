use betterlite_config::sections::settings::JsonValue;
use betterlite_db::database_faults::DbError;
use betterlite_db::database_faults::reconnect::{
    DbReconnect, RetryPolicy, call_with_db_reconnect_retry,
};
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_secret_managers::secret_types::KeyManagementSystem;

use super::SettingsReloadError;
use super::SettingsReloader;
use super::ports::SettingsRows;
use crate::config_override_support::decode::decode_stored_override;
use crate::config_override_support::environment::project_environment;
use crate::config_override_support::models::OverrideKind;
use crate::config_override_support::state::{
    clear_cyberark_state, clear_hashicorp_vault_state, snapshot_cyberark_boot_env,
};
use crate::env_value_codec::decrypt_env;

const HASHICORP_LOOKUP_REASON: &str = "init_hashicorp_vault_config_override_lookup_failure";
const CYBERARK_LOOKUP_REASON: &str = "init_cyberark_config_override_lookup_failure";

// One reload never lets a bad row stop the tick: the row's error only reaches this manager's log.
pub(super) async fn apply_override<Db, Client>(
    reloader: &mut SettingsReloader<Client>,
    db: &Db,
    kind: OverrideKind,
) -> Result<(), SettingsReloadError>
where
    Db: SettingsRows + ?Sized,
    Client: DbReconnect + Send,
{
    let Some(raw) =
        read_override(reloader.reconnect.as_mut(), db, kind, lookup_reason(kind)).await?
    else {
        if last_override(reloader, kind).is_some() {
            match kind {
                OverrideKind::HashicorpVault => clear_hashicorp_vault_state(
                    &mut reloader.environment,
                    &mut reloader.state,
                    reloader.control.as_mut(),
                ),
                OverrideKind::CyberArk => clear_cyberark_state(
                    &mut reloader.environment,
                    &mut reloader.state,
                    reloader.control.as_mut(),
                ),
            }
        }
        return Ok(());
    };
    let config = decode_stored_override(&raw, kind)?;
    if matches!(
        last_override(reloader, kind),
        Some(JsonValue::Object(previous)) if *previous == config
    ) {
        return Ok(());
    }
    apply_config(reloader, kind, config)?;
    match kind {
        OverrideKind::HashicorpVault => {
            tracing::debug!("Hashicorp Vault config override loaded from DB");
        }
        OverrideKind::CyberArk => {
            tracing::debug!("CyberArk config override loaded from DB");
        }
    }
    Ok(())
}

fn apply_config<Client: Send>(
    reloader: &mut SettingsReloader<Client>,
    kind: OverrideKind,
    config: JsonMap,
) -> Result<(), SettingsReloadError> {
    let decrypted = decrypt_env(&config, &reloader.cipher);
    if matches!(kind, OverrideKind::CyberArk) {
        snapshot_cyberark_boot_env(&mut reloader.state, &reloader.environment.snapshot());
    }
    let previous = snapshot_mapping(reloader, kind);
    reloader
        .environment
        .apply(&project_environment(kind, &decrypted));
    if let Err(source) = reloader.control.initialize(manager_of(kind)) {
        reloader
            .environment
            .apply(&project_environment(kind, &previous));
        return Err(SettingsReloadError::SecretManager(source));
    }
    let stored = JsonValue::Object(config);
    match kind {
        OverrideKind::HashicorpVault => reloader.state.vault_last_config = Some(stored),
        OverrideKind::CyberArk => reloader.state.cyberark_last_config = Some(stored),
    }
    Ok(())
}

fn last_override<Client: Send>(
    reloader: &SettingsReloader<Client>,
    kind: OverrideKind,
) -> Option<&JsonValue> {
    match kind {
        OverrideKind::HashicorpVault => reloader.state.vault_last_config.as_ref(),
        OverrideKind::CyberArk => reloader.state.cyberark_last_config.as_ref(),
    }
}

fn snapshot_mapping<Client: Send>(
    reloader: &SettingsReloader<Client>,
    kind: OverrideKind,
) -> JsonMap {
    let snapshot = reloader.environment.snapshot();
    let mut mapping = JsonMap::new();
    for field in kind.fields() {
        let stored = snapshot
            .values
            .get(field.env_var)
            .cloned()
            .map_or(JsonValue::Null, JsonValue::String);
        mapping.insert(field.name.to_owned(), stored);
    }
    mapping
}

async fn read_override<Db, Client>(
    reconnect: Option<&mut Client>,
    db: &Db,
    kind: OverrideKind,
    reason: &'static str,
) -> Result<Option<JsonValue>, DbError>
where
    Db: SettingsRows + ?Sized,
    Client: DbReconnect + Send,
{
    call_with_db_reconnect_retry(
        reconnect,
        move || async move { db.config_override(kind).await },
        RetryPolicy {
            reason,
            retry_safe_error_types: None,
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await
}

const fn lookup_reason(kind: OverrideKind) -> &'static str {
    match kind {
        OverrideKind::HashicorpVault => HASHICORP_LOOKUP_REASON,
        OverrideKind::CyberArk => CYBERARK_LOOKUP_REASON,
    }
}

const fn manager_of(kind: OverrideKind) -> KeyManagementSystem {
    match kind {
        OverrideKind::HashicorpVault => KeyManagementSystem::HashicorpVault,
        OverrideKind::CyberArk => KeyManagementSystem::Cyberark,
    }
}

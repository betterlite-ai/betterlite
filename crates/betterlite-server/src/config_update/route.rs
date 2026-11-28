use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_config::sections::settings::{JsonValue, Section};
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;

use super::body::{DecodedConfigUpdate, decode_config_update};
use super::store::ConfigUpdateStore;
use super::{ConfigUpdateError, ConfigUpdateState, SettingsMap, merge, router_settings};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::auth::auth_identity::AuthContext;
use crate::config_audit::{ConfigChange, config_audit_log};
use crate::proxy_config_state::state::ProxyConfigState;
use crate::tracing_routes::router::unauthenticated;

#[derive(Serialize)]
struct UpdatedBody {
    message: &'static str,
}

type MergeFn = fn(&SettingsMap, &SettingsMap) -> SettingsMap;

struct UpdateContext<'context> {
    config: &'context ProxyConfigState,
    store: &'context dyn ConfigUpdateStore,
    state: &'context ConfigUpdateState,
    caller: &'context CallerIdentity,
}

// the identity LiteLLM reads off user_api_key_dict, from the caller's key: user_id and the hashed api_key
#[derive(Debug, Deserialize)]
struct AuditKeyView {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default, rename = "api_key")]
    hashed_api_key: Option<String>,
}

pub(super) async fn update_config(
    State(state): State<ConfigUpdateState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let decoded = match decode_config_update(&body) {
        Ok(decoded) => decoded,
        Err(error) => return error.into_response(),
    };
    match apply(&state, &context.principal, decoded).await {
        Ok(()) => Json(UpdatedBody {
            message: "Config updated successfully",
        })
        .into_response(),
        Err(error) => error.into_response(),
    }
}

async fn apply(
    state: &ConfigUpdateState,
    principal: &Principal,
    body: DecodedConfigUpdate,
) -> Result<(), ConfigUpdateError> {
    if !principal.roles.contains(&Role::PlatformAdmin) {
        return Err(ConfigUpdateError::NotAdmin);
    }
    if let Some(raw) = &body.raw_router_settings {
        router_settings::check(raw)?;
    }
    let Some(store) = state.database.as_deref() else {
        return Err(ConfigUpdateError::NoDatabase);
    };
    let caller = audit_caller(principal)?;
    let context = UpdateContext {
        config: &state.config,
        store,
        state,
        caller: &caller,
    };
    let general = body.general_settings.as_ref().map(merge::without_nulls);
    let router = router_section(&body)?;
    guard(
        &context,
        general.as_ref(),
        body.litellm_settings.as_ref(),
        router.as_ref(),
    )?;
    write_sections(&context, &body, general.as_ref(), router.as_ref()).await?;
    state.deployments.add_deployment().await;
    Ok(())
}

// router_settings_updates: the typed dump, then raw keys the dump lacks whose value is not null
fn router_section(body: &DecodedConfigUpdate) -> Result<Option<SettingsMap>, ConfigUpdateError> {
    let Some(settings) = &body.router_settings else {
        return Ok(None);
    };
    let typed = router_settings::typed_settings(settings)?;
    Ok(Some(router_settings::updates(
        typed,
        body.raw_router_settings.as_ref(),
    )))
}

// reject_config_owned_writes runs for the three sections before any row is written
fn guard(
    context: &UpdateContext<'_>,
    general: Option<&SettingsMap>,
    litellm: Option<&SettingsMap>,
    router: Option<&SettingsMap>,
) -> Result<(), ConfigUpdateError> {
    if let Some(sent) = general {
        context
            .config
            .reject_config_owned_writes(Section::GeneralSettings, sent)?;
    }
    if let Some(sent) = litellm {
        context
            .config
            .reject_config_owned_writes(Section::LitellmSettings, sent)?;
    }
    if let Some(sent) = router {
        context
            .config
            .reject_config_owned_writes(Section::RouterSettings, sent)?;
    }
    Ok(())
}

// _read_section, merge, _upsert_section and the spawned audit, in LiteLLM's section order
async fn write_sections(
    context: &UpdateContext<'_>,
    body: &DecodedConfigUpdate,
    general: Option<&SettingsMap>,
    router: Option<&SettingsMap>,
) -> Result<(), ConfigUpdateError> {
    apply_section(
        context,
        Section::GeneralSettings,
        general,
        merge::general_settings,
    )
    .await?;
    if let Some(sent) = body.environment_variables.as_ref() {
        let sealed = context.store.seal_environment(sent)?;
        apply_section(
            context,
            Section::EnvironmentVariables,
            Some(&sealed),
            merge::environment_variables,
        )
        .await?;
    }
    apply_section(
        context,
        Section::LitellmSettings,
        body.litellm_settings.as_ref(),
        merge::litellm_settings,
    )
    .await?;
    apply_section(
        context,
        Section::RouterSettings,
        router,
        merge::router_settings,
    )
    .await
}

// _read_section, merge, _upsert_section, then the spawned create_config_audit_log with both stored values
async fn apply_section(
    context: &UpdateContext<'_>,
    section: Section,
    values: Option<&SettingsMap>,
    merge: MergeFn,
) -> Result<(), ConfigUpdateError> {
    let Some(sent) = values else {
        return Ok(());
    };
    let existing = context.store.read_section(section).await?;
    let merged = merge(&existing, sent);
    context.store.upsert_section(section, &merged).await?;
    spawn_audit(context, section, existing, merged);
    Ok(())
}

fn spawn_audit(
    context: &UpdateContext<'_>,
    section: Section,
    before: SettingsMap,
    after: SettingsMap,
) {
    let sink = Arc::clone(&context.state.audit);
    let spawner = Arc::clone(&context.state.spawner);
    let param = section.name().to_owned();
    let before_value = JsonValue::Object(before.into_iter().collect());
    let after_value = JsonValue::Object(after.into_iter().collect());
    let caller = context.caller.clone();
    spawner.spawn(Box::pin(async move {
        config_audit_log(
            &sink,
            ConfigChange {
                param: param.as_str(),
                action: AuditAction::Updated,
                table: AuditTable::Config,
                before: Some(&before_value),
                after: Some(&after_value),
                caller: &caller,
            },
        )
        .await;
    }));
}

fn audit_caller(principal: &Principal) -> Result<CallerIdentity, ConfigUpdateError> {
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(CallerIdentity::default());
    };
    let view: AuditKeyView = serde_json::from_value(serde_json::to_value(key)?)?;
    Ok(CallerIdentity {
        user_id: view.user_id,
        api_key: view.hashed_api_key.map(SecretString::from),
        metadata: None,
        team_metadata: None,
    })
}

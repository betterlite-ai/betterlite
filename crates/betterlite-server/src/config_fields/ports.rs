use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use secrecy::SecretString;
use serde::Deserialize;

use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::sink::AuditEventSink;
use crate::config_overlay_port::ConfigOverlayPort;
use crate::plugin_routes::config::PluginRegistry;
use crate::plugin_routes::routes::caller_identity;
use crate::proxy_config_state::database::ConfigStateDatabase;
use crate::proxy_config_state::state::ProxyConfigState;

use super::error::ConfigFieldError;
use super::litellm_fields::UiLitellmField;

pub const PROXY_ADMIN_ROLE: &str = "proxy_admin";
pub const PROXY_ADMIN_VIEWER_ROLE: &str = "proxy_admin_viewer";

#[derive(Debug, thiserror::Error)]
pub enum ConfigFieldRowError {
    #[error("the config database is not reachable")]
    Unavailable,
    #[error("the stored config row could not be read")]
    Read,
}

// ConfigRepository.table.find_first(where={"param_name": name}): the stored row's param_value, decoded
#[async_trait]
pub trait ConfigFieldRows: Send + Sync {
    async fn config_param(&self, name: &str) -> Result<Option<JsonValue>, ConfigFieldRowError>;
}

// litellm's module attributes and _apply_ssrf_general_settings live in the running proxy, so the routes reach them here
pub trait ConfigFieldRuntime: Send + Sync {
    fn ui_litellm_field(&self, field: UiLitellmField) -> JsonValue;

    fn set_ui_litellm_field(&self, field: UiLitellmField, value: JsonValue);

    fn apply_ssrf_settings(&self, general_settings: &BTreeMap<String, JsonValue>);
}

// the optional database store: row reads for the two tables, and B8380's writer for the upserts
#[derive(Clone)]
pub struct ConfigFieldDatabase {
    pub rows: Arc<dyn ConfigFieldRows>,
    pub store: Arc<dyn ConfigStateDatabase>,
}

// user_api_key_dict for these routes: the role gates, plus the caller the config audit log records
#[derive(Clone, Debug)]
pub struct ConfigFieldCaller {
    pub user_id: Option<String>,
    pub user_role: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KeyView {
    api_key: Option<SecretString>,
}

impl ConfigFieldCaller {
    pub fn from_principal(principal: &Principal) -> Result<Self, serde_json::Error> {
        let identity = caller_identity(principal)?;
        let key = match principal.source_key.0.as_ref() {
            Some(key) => serde_json::from_value(serde_json::to_value(key)?)?,
            None => KeyView::default(),
        };
        Ok(Self {
            user_id: identity.user_id,
            user_role: identity.user_role,
            api_key: key.api_key,
        })
    }

    #[must_use]
    pub fn audit_identity(&self) -> CallerIdentity {
        CallerIdentity {
            user_id: self.user_id.clone(),
            api_key: self.api_key.clone(),
            metadata: None,
            team_metadata: None,
        }
    }
}

// which of LiteLLM's two gates a route applies, and whether its refusal names the caller's role
#[derive(Clone, Copy)]
pub enum AdminGate {
    View,
    ViewWithRole,
    Proxy,
    ProxyWithRole,
}

pub fn require_admin(caller: &ConfigFieldCaller, gate: AdminGate) -> Result<(), ConfigFieldError> {
    let role = caller.user_role.as_deref();
    let admin_view = matches!(role, Some(PROXY_ADMIN_ROLE | PROXY_ADMIN_VIEWER_ROLE));
    let proxy_admin = role == Some(PROXY_ADMIN_ROLE);
    let (allowed, with_role) = match gate {
        AdminGate::View => (admin_view, false),
        AdminGate::ViewWithRole => (admin_view, true),
        AdminGate::Proxy => (proxy_admin, false),
        AdminGate::ProxyWithRole => (proxy_admin, true),
    };
    if allowed {
        return Ok(());
    }
    let detail = if with_role {
        format!(
            "{}, your role={}",
            CommonProxyErrors::NotAllowedAccess.as_str(),
            role.unwrap_or("None")
        )
    } else {
        CommonProxyErrors::NotAllowedAccess.as_str().to_owned()
    };
    Err(ConfigFieldError::BadRequest { detail })
}

// the prisma_client is None check every route runs first
pub fn require_database(
    database: Option<&ConfigFieldDatabase>,
) -> Result<&ConfigFieldDatabase, ConfigFieldError> {
    database.ok_or_else(|| ConfigFieldError::BadRequest {
        detail: CommonProxyErrors::DbNotConnectedError.as_str().to_owned(),
    })
}

// the stored param_value as the dict the routes edit; a non-object row is a stored-config error
pub fn object_entries(
    value: Option<&JsonValue>,
) -> Result<BTreeMap<String, JsonValue>, ConfigFieldError> {
    match value {
        None | Some(JsonValue::Null) => Ok(BTreeMap::new()),
        Some(JsonValue::Object(entries)) => Ok(entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect()),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_),
        ) => {
            tracing::error!("the stored config row is not a JSON object");
            Err(ConfigFieldError::Internal)
        }
    }
}

// everything one of the four services may touch, built once by the handler or by B8612
pub struct ConfigFieldServices<'run> {
    pub state: &'run mut ProxyConfigState,
    pub database: Option<&'run ConfigFieldDatabase>,
    pub runtime: &'run dyn ConfigFieldRuntime,
    pub plugins: &'run PluginRegistry,
    pub audit: &'run AuditEventSink,
    pub overlay: &'run dyn ConfigOverlayPort,
}

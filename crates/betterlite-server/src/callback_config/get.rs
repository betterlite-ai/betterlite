use std::collections::{BTreeMap, BTreeSet};

use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::settings::settings_resolution::FieldDescriptor;
use betterlite_config::sections::settings::settings_resolution::alerting::{
    EMAIL_DESCRIPTORS, MS_TEAMS_DESCRIPTORS, SLACK_DESCRIPTORS,
};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::{Identity, PROXY_ADMIN_ROLE, caller_identity};
use crate::proxy_config_state::snapshot::ConfigWithBaseline;
use crate::tracing_routes::router::unauthenticated;

use super::HandlerState;
use super::alerts::{self, AlertDestination, SecretKeys};
use super::callbacks::available::UiCallback;
use super::callbacks::{
    self, ALERTING_PATH, CALLBACKS, CALLBACKS_PATH, CallbackRow, ENVIRONMENT_VARIABLES,
    FAILURE_CALLBACK, FAILURE_PATH, GENERAL_SETTINGS, LITELLM_SETTINGS, SUCCESS_CALLBACK,
    SUCCESS_PATH,
};
use super::error::CallbackConfigError;
use super::hidden;

const SLACK: &str = "slack";
const SUCCESS: &str = "success";
const FAILURE: &str = "failure";
const SUCCESS_AND_FAILURE: &str = "success_and_failure";

#[derive(Serialize)]
pub(crate) struct GetReply {
    status: &'static str,
    callbacks: Vec<CallbackRow>,
    alerts: Vec<AlertDestination>,
    active_alerting_destinations: JsonValue,
    router_settings: BTreeMap<String, JsonValue>,
    available_callbacks: BTreeMap<&'static str, UiCallback>,
}

pub(crate) async fn get_config(
    State(state): State<HandlerState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let identity = match caller_identity(&context.principal) {
        Ok(identity) => identity,
        Err(error) => {
            return CallbackConfigError::Failure {
                message: error.to_string(),
            }
            .into_get_response();
        }
    };
    match config_callbacks(&state, &identity).await {
        Ok(reply) => Json(reply).into_response(),
        Err(error) => error.into_get_response(),
    }
}

struct Sections {
    litellm: BTreeMap<String, JsonValue>,
    general: BTreeMap<String, JsonValue>,
    environment: BTreeMap<String, JsonValue>,
}

fn read_sections(config: &ConfigWithBaseline) -> Result<Sections, CallbackConfigError> {
    Ok(Sections {
        litellm: callbacks::section_object(config.values.get(LITELLM_SETTINGS), LITELLM_SETTINGS)?,
        general: callbacks::section_object(config.values.get(GENERAL_SETTINGS), GENERAL_SETTINGS)?,
        environment: callbacks::section_object(
            config.values.get(ENVIRONMENT_VARIABLES),
            ENVIRONMENT_VARIABLES,
        )?,
    })
}

// The stored environment_variables section and the proxy's own process environment, in LiteLLM's order
struct VarSource<'stored> {
    stored: &'stored BTreeMap<String, JsonValue>,
    process: &'stored BTreeMap<String, String>,
}

struct ConfiguredCallbacks {
    success: Vec<String>,
    failure: Vec<String>,
    both: Vec<String>,
}

fn configured_callbacks(
    litellm: &BTreeMap<String, JsonValue>,
) -> Result<ConfiguredCallbacks, CallbackConfigError> {
    Ok(ConfiguredCallbacks {
        success: callbacks::normalized_entries(litellm.get(SUCCESS_CALLBACK), SUCCESS_PATH)?,
        failure: callbacks::normalized_entries(litellm.get(FAILURE_CALLBACK), FAILURE_PATH)?,
        both: callbacks::normalized_entries(litellm.get(CALLBACKS), CALLBACKS_PATH)?,
    })
}

fn configured_rows(configured: &ConfiguredCallbacks, source: &VarSource<'_>) -> Vec<CallbackRow> {
    let lists = [
        (&configured.success, SUCCESS),
        (&configured.failure, FAILURE),
        (&configured.both, SUCCESS_AND_FAILURE),
    ];
    let mut rows = Vec::new();
    for (names, callback_type) in lists {
        for name in names {
            rows.push(callbacks::process_callback(
                name,
                callback_type,
                source.stored,
                source.process,
            ));
        }
    }
    rows
}

fn configured_names(configured: &ConfiguredCallbacks) -> BTreeSet<String> {
    let names = configured
        .success
        .iter()
        .chain(&configured.failure)
        .chain(&configured.both);
    names
        .map(|name| callbacks::normalize_alias(name).to_owned())
        .collect()
}

// get_callbacks_by_type, minus the hidden names and the ones a configured name already covers
fn runtime_rows(
    state: &HandlerState,
    configured: &ConfiguredCallbacks,
    source: &VarSource<'_>,
) -> Vec<CallbackRow> {
    let reserved = configured_names(configured);
    let hidden_set = hidden::hidden_names(state.config.callbacks.as_ref(), &reserved);
    let selection = state.config.callbacks.selection();
    let lists = [
        (&selection.success, SUCCESS),
        (&selection.failure, FAILURE),
        (&selection.success_and_failure, SUCCESS_AND_FAILURE),
    ];
    let mut runtime = BTreeSet::new();
    for (names, callback_type) in lists {
        for name in names {
            if !hidden_set.contains(name) {
                runtime.insert((callbacks::normalize_alias(name).to_owned(), callback_type));
            }
        }
    }
    let mut rows = Vec::new();
    for (name, callback_type) in runtime {
        if !reserved.contains(&name) {
            let mut row =
                callbacks::process_callback(&name, callback_type, source.stored, source.process);
            row.read_only = Some(true);
            rows.push(row);
        }
    }
    rows
}

fn callback_rows(
    state: &HandlerState,
    sections: &Sections,
    is_full_admin: bool,
) -> Result<Vec<CallbackRow>, CallbackConfigError> {
    let configured = configured_callbacks(&sections.litellm)?;
    let source = VarSource {
        stored: &sections.environment,
        process: state.config.environment.as_ref(),
    };
    let mut rows = configured_rows(&configured, &source);
    rows.extend(runtime_rows(state, &configured, &source));
    if !is_full_admin {
        let keys = alerts::secret_keys();
        for row in &mut rows {
            row.variables = alerts::redact(&row.variables, &keys.callback);
        }
    }
    Ok(rows)
}

fn alerting_destinations(
    general: &BTreeMap<String, JsonValue>,
) -> Result<(bool, JsonValue), CallbackConfigError> {
    let alerting = callbacks::strict_entries(general.get("alerting"), ALERTING_PATH)?;
    let slack_enabled = alerting.iter().any(|destination| destination == SLACK);
    let destinations = alerting.into_iter().map(JsonValue::String).collect();
    Ok((slack_enabled, JsonValue::Array(destinations)))
}

struct AlertContext<'state> {
    sections: &'state Sections,
    environment: &'state BTreeMap<String, String>,
    keys: &'state SecretKeys,
    is_full_admin: bool,
}

fn destination(
    context: &AlertContext<'_>,
    name: &'static str,
    descriptors: &[FieldDescriptor],
) -> AlertDestination {
    AlertDestination {
        name,
        variables: alerts::alert_variables(
            descriptors,
            &context.sections.environment,
            context.environment,
            context.is_full_admin,
            context.keys,
        ),
        active_alerts: None,
        alerts_to_webhook: None,
    }
}

fn alert_destinations(
    state: &HandlerState,
    context: &AlertContext<'_>,
    slack_enabled: bool,
) -> Vec<AlertDestination> {
    let mut entries = Vec::new();
    if slack_enabled {
        let slack = state.config.alerting.as_ref();
        entries.push(AlertDestination {
            name: SLACK,
            variables: alerts::alert_variables(
                &SLACK_DESCRIPTORS,
                &context.sections.environment,
                context.environment,
                context.is_full_admin,
                context.keys,
            ),
            active_alerts: Some(alerts::active_alerts(&slack.active_alert_types())),
            alerts_to_webhook: Some(alerts::webhook_map(
                &slack.alert_to_webhook_url(),
                context.is_full_admin,
            )),
        });
    }
    entries.push(destination(context, "email", &EMAIL_DESCRIPTORS));
    entries.push(destination(context, "ms_teams", &MS_TEAMS_DESCRIPTORS));
    entries
}

async fn config_callbacks(
    state: &HandlerState,
    identity: &Identity,
) -> Result<GetReply, CallbackConfigError> {
    let is_full_admin = identity.user_role.as_deref() == Some(PROXY_ADMIN_ROLE);
    let config = state.config.overlay.reload_config().await?;
    let sections = read_sections(&config)?;
    let rows = callback_rows(state, &sections, is_full_admin)?;
    let (slack_enabled, active_alerting_destinations) = alerting_destinations(&sections.general)?;
    let keys = alerts::secret_keys();
    let context = AlertContext {
        sections: &sections,
        environment: state.config.environment.as_ref(),
        keys: &keys,
        is_full_admin,
    };
    let alerts_entries = alert_destinations(state, &context, slack_enabled);
    let router_settings = match &state.config.router {
        Some(reader) => reader.get_settings(),
        None => BTreeMap::new(),
    };
    Ok(GetReply {
        status: "success",
        callbacks: rows,
        alerts: alerts_entries,
        active_alerting_destinations,
        router_settings,
        available_callbacks: callbacks::available_callbacks(),
    })
}

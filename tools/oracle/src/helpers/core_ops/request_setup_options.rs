use std::collections::BTreeMap;

use betterlite_auth::auxiliary_routes::MCP_ROUTES;
use betterlite_auth::route_match::classifier::is_llm_api_route;
use betterlite_auth::route_match::templates::{RouteValue, check_route_access};
use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_metadata_safety::metadata::MetadataField;
use betterlite_core::api::request_metadata_safety::session::apply_missing_session_id_policy;
use betterlite_core::api::request_principal_metadata::team_callbacks::DefaultTeamSettings;
use betterlite_core::api::request_principal_metadata::{Metadata, PrincipalFacts};
use betterlite_core::api::request_setup_options::callbacks::CallbackSettings;
use betterlite_core::api::request_setup_options::destinations::disabled_backends;
use betterlite_core::api::request_setup_options::{
    CallbackVarsDecryption, RandomSessionIds, RequestSetupOptions, RequestSetupOptionsInput,
    RequestSetupSettings, SetupDiagnostic,
};
use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

const WARNING: &str = "WARNING";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    prepare: Vec<PrepareCall>,
    destinations: Vec<DestinationsCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareCall {
    data: RequestData,
    slot: String,
    route: String,
    query: Vec<(String, String)>,
    user_agent: String,
    missing_session_id: Option<String>,
    principal: PrincipalFacts,
    default_team_settings: Vec<DefaultTeamSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DestinationsCall {
    principal: PrincipalFacts,
    headers: Vec<(String, String)>,
    allow_dynamic_callback_disabling: bool,
    otel_v2: bool,
}

// the cases store callback_vars in plaintext, which LiteLLM's decrypt_callback_vars passes through unchanged
struct Plaintext;

impl CallbackVarsDecryption for Plaintext {
    fn decrypt(&self, metadata: &Metadata) -> Metadata {
        metadata.clone()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let prepared: Vec<Value> = input.prepare.iter().map(prepare_value).collect();
        let resolved: Vec<Value> = input.destinations.iter().map(destinations_value).collect();
        Ok(json!({ "prepare": prepared, "destinations": resolved }))
    }
}

fn service(
    settings: RequestSetupSettings,
) -> RequestSetupOptions<RandomSessionIds, Plaintext, BTreeMap<String, String>> {
    RequestSetupOptions {
        settings,
        session_ids: RandomSessionIds,
        decryption: Plaintext,
        environment: BTreeMap::new(),
    }
}

fn prepare_value(call: &PrepareCall) -> Value {
    let policy = call.missing_session_id.as_deref().map(|name| match name {
        "generate" => MissingSessionIdPolicy::Generate,
        "reject" => MissingSessionIdPolicy::Reject,
        "omit" => MissingSessionIdPolicy::Omit,
        other => MissingSessionIdPolicy::Unrecognized(other.to_owned()),
    });
    let slot = if call.slot == "litellm_metadata" {
        MetadataField::LitellmMetadata
    } else {
        MetadataField::Metadata
    };
    let route = call.route.as_str();
    let mut data = call.data.clone();
    apply_missing_session_id_policy(
        &mut data,
        slot,
        policy.as_ref(),
        is_llm_api_route(&RouteValue::Text(route)) && !check_route_access(route, MCP_ROUTES),
    );
    let options = service(RequestSetupSettings {
        allow_dynamic_callback_disabling: true,
        missing_session_id: policy,
        otel_v2_enabled: false,
        default_team_settings: call.default_team_settings.clone(),
        allowed_hosts: Vec::new(),
    });
    let input = RequestSetupOptionsInput {
        data,
        route: call.route.clone(),
        query: call.query.clone(),
        user_agent: call.user_agent.clone(),
        metadata_variable_name: slot,
        principal: call.principal.clone(),
    };
    match options.prepare(input) {
        Ok(result) => json!({
            "outcome": {
                "data": result.data,
                "callback_settings": result.callback_settings.as_ref().map(settings_value),
            },
            "warnings": warning_levels(&result.diagnostics),
        }),
        Err(error) => {
            let facts = error.facts();
            json!({ "error": {
                "status": facts.status,
                "type": facts.error_type,
                "param": facts.param,
                "code": facts.code,
            }})
        }
    }
}

fn settings_value(settings: &CallbackSettings) -> Value {
    let callback_vars: IndexMap<&str, &str> = settings
        .callback_vars
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    json!({
        "success_callback": settings.success_callback,
        "failure_callback": settings.failure_callback,
        "callbacks": settings.callbacks,
        "callback_vars": callback_vars,
    })
}

fn destinations_value(call: &DestinationsCall) -> Value {
    let options = service(RequestSetupSettings {
        allow_dynamic_callback_disabling: call.allow_dynamic_callback_disabling,
        missing_session_id: None,
        otel_v2_enabled: call.otel_v2,
        default_team_settings: Vec::new(),
        allowed_hosts: Vec::new(),
    });
    let headers: IndexMap<String, String> = call.headers.iter().cloned().collect();
    let disabled = disabled_backends(
        call.allow_dynamic_callback_disabling,
        &call.principal.metadata,
        &headers,
    );
    let resolved = options.tenant_destinations(&call.principal, &headers);
    let destinations: Vec<Value> = resolved
        .destinations
        .iter()
        .map(|destination| {
            let fields = destination.as_ref();
            json!({
                "endpoint": fields.endpoint,
                "headers": fields.headers,
                "resource_attributes": fields.resource_attributes,
                "callback_name": fields.callback_name,
                "protocol": fields.protocol,
                "span_scope": match fields.span_scope {
                    SpanScope::Full => "full",
                    SpanScope::LlmOnly => "llm_only",
                },
            })
        })
        .collect();
    json!({
        "disabled": disabled,
        "destinations": destinations,
        "warnings": warning_levels(&resolved.diagnostics),
    })
}

// LiteLLM's wording is its own, so the comparison reads the level of each warning in order
fn warning_levels(diagnostics: &[SetupDiagnostic]) -> Vec<&'static str> {
    diagnostics
        .iter()
        .map(|diagnostic| match diagnostic {
            SetupDiagnostic::UnknownSessionPolicy { .. }
            | SetupDiagnostic::InvalidCallbackMetadata { .. } => WARNING,
        })
        .collect()
}

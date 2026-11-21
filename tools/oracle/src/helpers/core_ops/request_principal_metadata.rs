use std::collections::BTreeMap;

use betterlite_auth::agent_policy::ManagedAgentContext;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_metadata_safety::metadata::MetadataField;
use betterlite_core::api::request_principal_metadata::key_controls::add_key_level_controls;
use betterlite_core::api::request_principal_metadata::principal::add_principal_metadata;
use betterlite_core::api::request_principal_metadata::tags::request_tags;
use betterlite_core::api::request_principal_metadata::team_callbacks::{
    DefaultTeamSettings, team_callbacks,
};
use betterlite_core::api::request_principal_metadata::{
    Metadata, PrincipalFacts, RequestPrincipalMetadataInput, apply_pre_auth_tags,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pre_auth_tags: Vec<PreAuthCall>,
    request_metadata: Vec<MetadataCall>,
    team_callbacks: Vec<TeamCall>,
}

#[derive(Debug, Deserialize)]
struct PreAuthCall {
    data: RequestData,
    headers: Vec<(String, String)>,
    route: String,
    principal: PrincipalFacts,
}

#[derive(Debug, Deserialize)]
struct MetadataCall {
    data: RequestData,
    slot: String,
    bucket: Metadata,
    headers: Vec<(String, String)>,
    principal: PrincipalFacts,
    invoked_agent_id: Option<String>,
    managed_agent_context: Option<ManagedAgentContext>,
}

#[derive(Debug, Deserialize)]
struct TeamCall {
    team_id: String,
    teams: Vec<DefaultTeamSettings>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let pre_auth: Vec<Value> = input.pre_auth_tags.iter().map(pre_auth_value).collect();
        let metadata: Vec<Value> = input.request_metadata.iter().map(metadata_value).collect();
        let teams: Vec<Value> = input.team_callbacks.iter().map(team_value).collect();
        Ok(json!({
            "pre_auth_tags": pre_auth,
            "request_metadata": metadata,
            "team_callbacks": teams,
        }))
    }
}

fn pre_auth_value(call: &PreAuthCall) -> Value {
    let input = RequestPrincipalMetadataInput {
        data: call.data.clone(),
        headers: call.headers.iter().cloned().collect(),
        route: call.route.clone(),
        principal: call.principal.clone(),
    };
    json!({ "data": apply_pre_auth_tags(&input, ApiLimits::default()).data })
}

fn metadata_value(call: &MetadataCall) -> Value {
    let slot = if call.slot == "litellm_metadata" {
        MetadataField::LitellmMetadata
    } else {
        MetadataField::Metadata
    };
    let mut principal = call.principal.clone();
    principal
        .invoked_agent_id
        .clone_from(&call.invoked_agent_id);
    principal
        .managed_agent_context
        .clone_from(&call.managed_agent_context);
    let mut data = call.data.clone();
    data.insert(call.slot.clone(), Json::Object(call.bucket.clone()));
    let auth_view = match add_principal_metadata(&mut data, slot, &principal) {
        Ok(view) => view,
        Err(error) => return json!({ "error": error.to_string() }),
    };
    add_key_level_controls(&mut data, slot, &principal.metadata);
    let headers = call.headers.iter().cloned().collect();
    json!({
        "data": data,
        "auth_view": {
            "metadata": auth_view.metadata,
            "team_metadata": auth_view.team_metadata,
            "project_metadata": auth_view.project_metadata,
            "organization_metadata": auth_view.organization_metadata,
        },
        "request_tags": request_tags(&headers, &data),
    })
}

fn team_value(call: &TeamCall) -> Value {
    match team_callbacks(
        &call.team_id,
        &call.teams,
        &BTreeMap::<String, String>::new(),
    ) {
        Ok(Some(callbacks)) => {
            let callback_vars: BTreeMap<&str, &str> = callbacks
                .callback_vars
                .iter()
                .map(|(name, value)| (name.as_str(), value.expose_secret()))
                .collect();
            json!({
                "success_callback": callbacks.success_callback,
                "failure_callback": callbacks.failure_callback,
                "callback_vars": callback_vars,
            })
        }
        Ok(None) => Value::Null,
        Err(_error) => json!({ "error": "invalid callback variable" }),
    }
}

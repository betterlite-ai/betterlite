use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use betterlite_core::api::request_guardrail_policy::{
    GuardrailMetadata, GuardrailPolicyService, GuardrailPolicySettings, GuardrailPrincipal,
    RequestGuardrailPolicyInput,
};
use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::registry::{PolicyRegistry, RawPolicy, RegistryError};
use betterlite_policy::values::PolicyAttachment;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    data: RequestData,
    #[serde(default)]
    key_metadata: GuardrailMetadata,
    #[serde(default)]
    team_metadata: GuardrailMetadata,
    #[serde(default)]
    project_metadata: GuardrailMetadata,
    team_alias: Option<String>,
    key_alias: Option<String>,
    metadata_variable_name: String,
    premium_user: bool,
    policies: Option<BTreeMap<String, RawPolicy>>,
    attachments: Option<Vec<PolicyAttachment>>,
    max_items: Option<usize>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(call_value(call)?);
        }
        Ok(Value::Array(results))
    }
}

fn call_value(call: &Call) -> HelperResult {
    let mut registry = PolicyRegistry::default();
    if let Some(policies) = &call.policies {
        registry
            .load_policies(policies.clone())
            .map_err(|error| policy_error(&error))?;
    }
    let mut attachments = AttachmentRegistry::default();
    if let Some(entries) = &call.attachments {
        attachments.load_attachments(entries.clone());
    }
    let defaults = ApiLimits::default();
    let service = GuardrailPolicyService {
        policies: &registry,
        attachments: &attachments,
        settings: GuardrailPolicySettings {
            premium_user: call.premium_user,
            limits: ApiLimits {
                max_json_bytes: defaults.max_json_bytes,
                max_event_bytes: defaults.max_event_bytes,
                max_items: call.max_items.unwrap_or(defaults.max_items),
            },
        },
    };
    match service.resolve(&request_input(call)) {
        Ok(result) => {
            let mut excluded = result.excluded_policies;
            excluded.sort();
            Ok(json!({"data": result.data, "excluded": excluded}))
        }
        Err(error) => Ok(error_facts(&error)),
    }
}

fn request_input(call: &Call) -> RequestGuardrailPolicyInput {
    RequestGuardrailPolicyInput {
        data: call.data.clone(),
        principal: GuardrailPrincipal {
            team_alias: call.team_alias.clone(),
            key_alias: call.key_alias.clone(),
        },
        key_metadata: call.key_metadata.clone(),
        team_metadata: call.team_metadata.clone(),
        project_metadata: call.project_metadata.clone(),
        metadata_variable_name: call.metadata_variable_name.clone(),
    }
}

// the Python side maps the same HTTPException through proxy_exception_from_http_exception
fn error_facts(error: &RequestGuardrailPolicyError) -> Value {
    let facts = error.facts();
    json!({
        "status": facts.status,
        "type": facts.error_type,
        "param": facts.param,
        "code": facts.code,
    })
}

fn policy_error(error: &RegistryError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

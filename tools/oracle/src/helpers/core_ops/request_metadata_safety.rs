use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_metadata_safety::error::ErrorFacts;
use betterlite_core::api::request_metadata_safety::{
    MetadataPrincipal, MetadataSafetySettings, RequestMetadataSafetyInput,
    sanitize_request_metadata,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<SanitizeCall>,
}

#[derive(Debug, Deserialize)]
struct SanitizeCall {
    data: RequestData,
    headers: Vec<(String, String)>,
    route: String,
    principal: SanitizePrincipal,
    settings: SanitizeSettings,
}

#[derive(Debug, Deserialize)]
struct SanitizePrincipal {
    #[serde(default)]
    metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    team_metadata: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
struct SanitizeSettings {
    #[serde(default)]
    missing_session_id: Option<String>,
    #[serde(default)]
    turn_off_message_logging: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(sanitize_value(call)?);
        }
        Ok(Value::Array(results))
    }
}

fn sanitize_value(call: &SanitizeCall) -> HelperResult {
    let input = RequestMetadataSafetyInput {
        data: call.data.clone(),
        headers: call.headers.iter().cloned().collect(),
        route: call.route.clone(),
        principal: MetadataPrincipal {
            api_key: None,
            via_virtual_key: false,
            metadata: call.principal.metadata.clone(),
            team_metadata: call.principal.team_metadata.clone(),
        },
    };
    let settings = MetadataSafetySettings {
        limits: ApiLimits::default(),
        missing_session_id: policy(call.settings.missing_session_id.as_deref()),
        turn_off_message_logging: call.settings.turn_off_message_logging,
    };
    match sanitize_request_metadata(&input, &settings) {
        Ok(result) => Ok(json!({
            "data": result.data,
            "headers": header_pairs(&result.headers),
            "metadata_variable_name": result.metadata_variable_name.as_str(),
        })),
        Err(error) => Ok(json!({ "error": facts_json(error.facts()) })),
    }
}

fn policy(name: Option<&str>) -> Option<MissingSessionIdPolicy> {
    name.map(|text| match text {
        "generate" => MissingSessionIdPolicy::Generate,
        "reject" => MissingSessionIdPolicy::Reject,
        "omit" => MissingSessionIdPolicy::Omit,
        other => MissingSessionIdPolicy::Unrecognized(other.to_owned()),
    })
}

fn facts_json(facts: ErrorFacts) -> Value {
    json!({
        "status": facts.status,
        "type": facts.error_type,
        "param": facts.param,
        "code": facts.code,
    })
}

fn header_pairs(headers: &IndexMap<String, String>) -> Vec<(&str, &str)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

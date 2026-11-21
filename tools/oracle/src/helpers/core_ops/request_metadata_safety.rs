use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_metadata_safety::admin::{
    ALLOW_CLIENT_MOCK_RESPONSE_KEY, key_or_team_metadata_flag_is_true,
};
use betterlite_core::api::request_metadata_safety::error::ErrorFacts;
use betterlite_core::api::request_metadata_safety::identity::{
    anthropic_session_id_from_metadata, get_chain_id_from_headers, sanitize_for_log,
    stampable_key_hash,
};
use betterlite_core::api::request_metadata_safety::metadata::{
    MetadataField, metadata_variable_name_for_route, normalized_metadata_slot,
    promoted_trace_control_fields,
};
use betterlite_core::api::request_metadata_safety::{
    MetadataPrincipal, MetadataSafetySettings, RequestMetadataSafetyInput,
    sanitize_request_metadata,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Sanitize(SanitizeCall),
    ChainId(HeadersCall),
    AnthropicSessionId(MetadataCall),
    StampableKeyHash(HashCall),
    SanitizeForLog(ValueCall),
    PromotedTraceControlFields(PromotedCall),
    MetadataVariableName(RouteCall),
    KeyOrTeamFlag(FlagCall),
    MockResponseFlag(PrincipalCall),
    NormalizedMetadataSlot(SlotCall),
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

#[derive(Debug, Deserialize)]
struct HeadersCall {
    headers: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
struct MetadataCall {
    metadata: Json,
}

#[derive(Debug, Deserialize)]
struct HashCall {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    via_virtual_key: bool,
}

#[derive(Debug, Deserialize)]
struct ValueCall {
    value: Json,
}

#[derive(Debug, Deserialize)]
struct PromotedCall {
    requester_metadata: IndexMap<String, Json>,
    litellm_metadata: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
struct RouteCall {
    route: String,
}

#[derive(Debug, Deserialize)]
struct FlagCall {
    metadata_key: String,
    #[serde(default)]
    metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    team_metadata: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
struct PrincipalCall {
    #[serde(default)]
    metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    team_metadata: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
struct SlotCall {
    data: RequestData,
    metadata_variable_name: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call)?);
        }
        Ok(Value::Array(results))
    }
}

fn run_call(entry: &Call) -> HelperResult {
    match entry {
        Call::Sanitize(call) => sanitize_value(call),
        Call::ChainId(call) => Ok(optional_string(get_chain_id_from_headers(&headers_map(
            &call.headers,
        )))),
        Call::AnthropicSessionId(call) => Ok(optional_string(anthropic_session_id_from_metadata(
            &call.metadata,
        ))),
        Call::StampableKeyHash(call) => {
            Ok(optional_string(stampable_key_hash(&hash_principal(call))))
        }
        Call::SanitizeForLog(call) => Ok(Value::String(sanitize_for_log(&call.value))),
        Call::PromotedTraceControlFields(call) => Ok(serde_json::to_value(
            promoted_trace_control_fields(&call.requester_metadata, &call.litellm_metadata),
        )?),
        Call::MetadataVariableName(call) => Ok(Value::String(
            metadata_variable_name_for_route(&call.route)
                .as_str()
                .to_owned(),
        )),
        Call::KeyOrTeamFlag(call) => Ok(Value::Bool(key_or_team_metadata_flag_is_true(
            &flag_principal(call.metadata.as_ref(), call.team_metadata.as_ref()),
            &call.metadata_key,
        ))),
        Call::MockResponseFlag(call) => Ok(Value::Bool(key_or_team_metadata_flag_is_true(
            &flag_principal(call.metadata.as_ref(), call.team_metadata.as_ref()),
            ALLOW_CLIENT_MOCK_RESPONSE_KEY,
        ))),
        Call::NormalizedMetadataSlot(call) => normalized_slot_value(call),
    }
}

fn sanitize_value(call: &SanitizeCall) -> HelperResult {
    let input = RequestMetadataSafetyInput {
        data: call.data.clone(),
        headers: headers_map(&call.headers),
        route: call.route.clone(),
        principal: MetadataPrincipal {
            metadata: call.principal.metadata.clone(),
            team_metadata: call.principal.team_metadata.clone(),
            ..MetadataPrincipal::default()
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

fn normalized_slot_value(call: &SlotCall) -> HelperResult {
    let field = field_named(&call.metadata_variable_name).or_value_error()?;
    let mut data = call.data.clone();
    normalized_metadata_slot(&mut data, field, ApiLimits::default());
    Ok(serde_json::to_value(data)?)
}

fn field_named(name: &str) -> Result<MetadataField, String> {
    match name {
        "metadata" => Ok(MetadataField::Metadata),
        "litellm_metadata" => Ok(MetadataField::LitellmMetadata),
        other => Err(format!("unknown metadata variable name: {other}")),
    }
}

fn policy(name: Option<&str>) -> Option<MissingSessionIdPolicy> {
    name.map(|name| match name {
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

fn headers_map(headers: &[(String, String)]) -> IndexMap<String, String> {
    headers.iter().cloned().collect()
}

fn hash_principal(call: &HashCall) -> MetadataPrincipal {
    MetadataPrincipal {
        api_key: call
            .api_key
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        via_virtual_key: call.via_virtual_key,
        ..MetadataPrincipal::default()
    }
}

fn flag_principal(
    metadata: Option<&IndexMap<String, Json>>,
    team_metadata: Option<&IndexMap<String, Json>>,
) -> MetadataPrincipal {
    MetadataPrincipal {
        metadata: metadata.cloned(),
        team_metadata: team_metadata.cloned(),
        ..MetadataPrincipal::default()
    }
}

fn optional_string(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::String)
}

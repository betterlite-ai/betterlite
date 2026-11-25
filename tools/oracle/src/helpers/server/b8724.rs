use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp::transport_checks::peek::{
    has_top_level_method, initialize_client_info, is_initialize_request,
};
use betterlite_server::mcp::transport_checks::routing::{
    forwarded_authorization, owner_fingerprint, servers_in_path,
};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Operation {
    ServerInPath { paths: Vec<String> },
    OwnerFingerprint { calls: Vec<FingerprintCall> },
    InitializeDetection { bodies: Vec<String> },
    ClientInfo { bodies: Vec<String> },
    TopLevelMethodScan { texts: Vec<String> },
    ForwardedAuthorization { headers: Vec<Vec<(String, String)>> },
}

// the UserAPIKeyAuth fields a case sets; construct hashes an api_key on both sides
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FingerprintCall {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    oauth2_headers: Option<Vec<(String, String)>>,
    #[serde(default)]
    client_ip: Option<String>,
}

fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::from_bytes(name.as_bytes()).or_value_error()?;
        let header_value = HeaderValue::from_bytes(value.as_bytes()).or_value_error()?;
        map.append(header_name, header_value);
    }
    Ok(map)
}

fn auth_for(call: &FingerprintCall) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(api_key) = &call.api_key {
        fields.insert("api_key".to_owned(), JsonData::String(api_key.clone()));
    }
    if let Some(user_id) = &call.user_id {
        fields.insert("user_id".to_owned(), JsonData::String(user_id.clone()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

fn fingerprint_answer(call: &FingerprintCall) -> Result<String, Box<HelperError>> {
    let auth = auth_for(call)?;
    let oauth2_headers = call
        .oauth2_headers
        .as_ref()
        .map(|pairs| pairs.iter().cloned().collect::<BTreeMap<String, String>>());
    Ok(owner_fingerprint(
        Some(&auth),
        oauth2_headers.as_ref(),
        call.client_ip.as_deref(),
    ))
}

fn client_info_answer(body: &str) -> Value {
    match initialize_client_info(body.as_bytes()) {
        Some(info) => json!({"name": info.name, "version": info.version}),
        None => Value::Null,
    }
}

fn path_answers(paths: &[String]) -> HelperResult {
    let answers = paths
        .iter()
        .map(|path| servers_in_path(path))
        .collect::<Vec<_>>();
    Ok(serde_json::to_value(answers)?)
}

fn fingerprint_answers(calls: &[FingerprintCall]) -> HelperResult {
    let answers = calls
        .iter()
        .map(fingerprint_answer)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::to_value(answers)?)
}

fn initialize_answers(bodies: &[String]) -> HelperResult {
    let answers = bodies
        .iter()
        .map(|body| is_initialize_request(body.as_bytes()))
        .collect::<Vec<_>>();
    Ok(serde_json::to_value(answers)?)
}

fn client_info_answers(bodies: &[String]) -> HelperResult {
    let answers = bodies
        .iter()
        .map(|body| client_info_answer(body))
        .collect::<Vec<_>>();
    Ok(serde_json::to_value(answers)?)
}

fn method_scan_answers(texts: &[String]) -> HelperResult {
    let answers = texts
        .iter()
        .map(|text| has_top_level_method(text))
        .collect::<Vec<_>>();
    Ok(serde_json::to_value(answers)?)
}

fn forwarded_answers(headers: &[Vec<(String, String)>]) -> HelperResult {
    let answers = headers
        .iter()
        .map(|pairs| header_map(pairs).map(|map| forwarded_authorization(&map)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::to_value(answers)?)
}

impl Helper for Op {
    type Input = Operation;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Operation::ServerInPath { paths } => path_answers(paths),
            Operation::OwnerFingerprint { calls } => fingerprint_answers(calls),
            Operation::InitializeDetection { bodies } => initialize_answers(bodies),
            Operation::ClientInfo { bodies } => client_info_answers(bodies),
            Operation::TopLevelMethodScan { texts } => method_scan_answers(texts),
            Operation::ForwardedAuthorization { headers } => forwarded_answers(headers),
        }
    }
}

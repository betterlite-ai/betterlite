use std::collections::BTreeMap;

use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_claims::handler::JwtHandler;
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::sso_claims::access_token::process_sso_jwt_access_token;
use betterlite_auth::sso_claims::convert::{
    ConversionConfig, convert_generic_response, normalize_email,
};
use betterlite_auth::sso_claims::decode::{decode_sso_token_claims, merge_sso_token_claims};
use betterlite_auth::sso_claims::roles::determine_role_from_groups;
use betterlite_auth::sso_types::identity::OpenIdIdentity;
use betterlite_auth::sso_types::records::{RoleMappings, TeamMappings};
use betterlite_config::sso_claims::{ClaimAttributes, load_claim_attributes};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    attributes: BTreeMap<String, String>,
    emails: Vec<Option<String>>,
    groups: Vec<Vec<String>>,
    role_mappings: Option<RoleMappings>,
    merge: Vec<MergeCall>,
    decode_tokens: Vec<Option<String>>,
    process: Vec<ProcessCall>,
    convert: ConvertCall,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeCall {
    userinfo: BTreeMap<String, TypedValue>,
    id_token: Option<String>,
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessCall {
    access_token: Option<String>,
    sso_settings: Option<JwtAuthSettings>,
    identity: Option<OpenIdIdentity>,
    role_mappings: Option<RoleMappings>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConvertCall {
    response: TypedValue,
    jwt_settings: JwtAuthSettings,
    sso_settings: Option<JwtAuthSettings>,
    role_mappings: Option<RoleMappings>,
    team_mappings: Option<TeamMappings>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let attributes = load_claim_attributes(&input.attributes);
        let claims = claim_results(input);
        let (processed, converted) = exchange_results(input, &attributes)?;
        Ok(json!({
            "emails": claims.emails,
            "roles": claims.roles,
            "decoded": claims.decoded,
            "merged": claims.merged,
            "processed": processed,
            "converted": converted,
        }))
    }
}

struct ClaimResults {
    emails: Vec<Option<String>>,
    roles: Vec<Option<String>>,
    decoded: Vec<BTreeMap<String, TypedValue>>,
    merged: Vec<BTreeMap<String, TypedValue>>,
}

fn claim_results(input: &Input) -> ClaimResults {
    ClaimResults {
        emails: input
            .emails
            .iter()
            .map(|email| normalize_email(email.as_deref()))
            .collect(),
        roles: input
            .groups
            .iter()
            .map(|groups| group_role(groups, input.role_mappings.as_ref()))
            .collect(),
        decoded: input
            .decode_tokens
            .iter()
            .map(|token| decode_sso_token_claims(token.as_deref()))
            .collect(),
        merged: input
            .merge
            .iter()
            .map(|call| {
                merge_sso_token_claims(
                    &call.userinfo,
                    call.id_token.as_deref(),
                    call.access_token.as_deref(),
                )
            })
            .collect(),
    }
}

fn exchange_results(
    input: &Input,
    attributes: &ClaimAttributes,
) -> Result<(Vec<Value>, OpenIdIdentity), Box<HelperError>> {
    let processed = input
        .process
        .iter()
        .map(|call| process_call(call, attributes))
        .collect::<Result<Vec<_>, _>>()?;
    let converted = convert_call(&input.convert, attributes)?;
    Ok((processed, converted))
}

fn handler(settings: &JwtAuthSettings) -> JwtHandler {
    let mut handler = JwtHandler::default();
    handler.update_environment(settings.clone(), Vec::new(), 0);
    handler
}

fn group_role(groups: &[String], mappings: Option<&RoleMappings>) -> Option<String> {
    let mappings = mappings?;
    let role = determine_role_from_groups(groups, mappings)?;
    Some(role.to_string())
}

fn process_call(call: &ProcessCall, attributes: &ClaimAttributes) -> HelperResult {
    let sso_jwt_handler = call.sso_settings.as_ref().map(handler);
    let (identity, payload) = process_sso_jwt_access_token(
        call.access_token.as_deref(),
        sso_jwt_handler.as_ref(),
        call.identity.clone(),
        call.role_mappings.as_ref(),
        &attributes.role,
    )
    .or_value_error()?;
    Ok(json!({"identity": identity, "payload": payload}))
}

fn convert_call(
    call: &ConvertCall,
    attributes: &ClaimAttributes,
) -> Result<OpenIdIdentity, Box<HelperError>> {
    let jwt_handler = handler(&call.jwt_settings);
    let sso_jwt_handler = call.sso_settings.as_ref().map(handler);
    let config = ConversionConfig {
        jwt_handler: &jwt_handler,
        sso_jwt_handler: sso_jwt_handler.as_ref(),
        role_mappings: call.role_mappings.as_ref(),
        team_mappings: call.team_mappings.as_ref(),
    };
    convert_generic_response(&call.response, &config, attributes).or_value_error()
}

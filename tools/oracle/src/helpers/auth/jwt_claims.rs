use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_claims::claims::JwtClaims;
use betterlite_auth::jwt_claims::decode::unverified_claims;
use betterlite_auth::jwt_claims::handler::JwtHandler;
use betterlite_auth::jwt_claims::identity::IdentityClaims as _;
use betterlite_auth::jwt_claims::orgs::OrgClaims as _;
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::jwt_claims::teams::TeamClaims as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
    tokens: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    settings: JwtAuthSettings,
    trusted_issuers: Vec<String>,
    claims: TypedValue,
    default_value: Option<String>,
    valid_user_email: Option<bool>,
    leeway: i64,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut scenarios = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            scenarios.push(project(scenario)?);
        }
        let unverified = input
            .tokens
            .iter()
            .map(|token| json!(unverified_claims(token)))
            .collect::<Vec<Value>>();
        Ok(json!({"scenarios": scenarios, "unverified": unverified}))
    }
}

fn project(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let mut handler = JwtHandler::default();
    handler.update_environment(
        scenario.settings.clone(),
        scenario.trusted_issuers.clone(),
        scenario.leeway,
    );
    let claims = JwtClaims {
        claims: scenario.claims.clone(),
        settings: &handler.settings,
        trusted_issuers: &handler.trusted_issuers,
    };
    let default = scenario.default_value.as_deref();
    Ok(json!({
        "team_id": claims.team_id(default).or_value_error()?,
        "team_ids": claims.team_ids().or_value_error()?,
        "all_team_ids": claims.all_team_ids().or_value_error()?,
        "team_alias": claims.team_alias(default).or_value_error()?,
        "end_user_id": claims.end_user_id(default).or_value_error()?,
        "user_id": claims.user_id(default).or_value_error()?,
        "user_email": claims.user_email(default).or_value_error()?,
        "object_id": claims.object_id(default).or_value_error()?,
        "agent_claim": claims.agent_claim().or_value_error()?,
        "org_id": claims.org_id(default).or_value_error()?,
        "org_alias": claims.org_alias(default).or_value_error()?,
        "is_required_team_id": claims.is_required_team_id(),
        "is_enforced_email_domain": claims.is_enforced_email_domain(),
        "is_upsert_user_id": claims.is_upsert_user_id(scenario.valid_user_email),
        "leeway": handler.leeway,
    }))
}

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::team_request_contracts::create::NewTeamRequest;
use crate::team_request_contracts::update::UpdateTeamRequest;
use crate::team_request_contracts::values::ThroughputLimitType;

// the NewTeamRequest | UpdateTeamRequest fields the parent ceilings read
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamLimitRequest<'request> {
    pub max_budget: Option<Decimal>,
    pub models: Option<Vec<JsonData>>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub metadata: Option<&'request BTreeMap<String, JsonData>>,
    pub model_rpm_limit: Option<&'request BTreeMap<String, i64>>,
    pub model_tpm_limit: Option<&'request BTreeMap<String, i64>>,
    pub rpm_limit_type: Option<ThroughputLimitType>,
    pub tpm_limit_type: Option<ThroughputLimitType>,
}

impl<'request> From<&'request NewTeamRequest> for TeamLimitRequest<'request> {
    fn from(request: &'request NewTeamRequest) -> Self {
        let team = &request.team;
        let extra = &request.extra;
        Self {
            max_budget: team.max_budget,
            models: Some(team.models.iter().cloned().map(JsonData::String).collect()),
            tpm_limit: team.tpm_limit,
            rpm_limit: team.rpm_limit,
            metadata: team.metadata.as_ref(),
            model_rpm_limit: extra.model_rpm_limit.as_ref(),
            model_tpm_limit: extra.model_tpm_limit.as_ref(),
            rpm_limit_type: extra.rpm_limit_type,
            tpm_limit_type: extra.tpm_limit_type,
        }
    }
}

// UpdateTeamRequest declares no rpm_limit_type or tpm_limit_type, so an update reads them from metadata
impl<'request> From<&'request UpdateTeamRequest> for TeamLimitRequest<'request> {
    fn from(request: &'request UpdateTeamRequest) -> Self {
        let fields = &request.fields;
        Self {
            max_budget: fields.max_budget.as_ref().map(|amount| amount.0),
            models: fields.models.clone(),
            tpm_limit: fields.tpm_limit,
            rpm_limit: fields.rpm_limit,
            metadata: fields.metadata.as_ref(),
            model_rpm_limit: fields.model_rpm_limit.as_ref(),
            model_tpm_limit: fields.model_tpm_limit.as_ref(),
            rpm_limit_type: None,
            tpm_limit_type: None,
        }
    }
}

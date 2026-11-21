use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::team_request_contracts::create::NewTeamRequest;
use crate::team_request_contracts::update::UpdateTeamRequest;
use crate::team_request_contracts::values::ThroughputLimitType;

// LiteLLM's Union[NewTeamRequest, UpdateTeamRequest], read field by field as its getattr calls do
#[derive(Debug, Clone, Copy)]
pub enum TeamLimitRequest<'request> {
    New(&'request NewTeamRequest),
    Update(&'request UpdateTeamRequest),
}

impl<'request> TeamLimitRequest<'request> {
    #[must_use]
    pub fn max_budget(&self) -> Option<Decimal> {
        match *self {
            Self::New(request) => request.team.max_budget,
            Self::Update(request) => request.fields.max_budget.as_ref().map(|amount| amount.0),
        }
    }

    #[must_use]
    pub fn models(&self) -> Option<Vec<JsonData>> {
        match *self {
            Self::New(request) => Some(
                request
                    .team
                    .models
                    .iter()
                    .cloned()
                    .map(JsonData::String)
                    .collect(),
            ),
            Self::Update(request) => request.fields.models.clone(),
        }
    }

    #[must_use]
    pub const fn tpm_limit(&self) -> Option<i64> {
        match *self {
            Self::New(request) => request.team.tpm_limit,
            Self::Update(request) => request.fields.tpm_limit,
        }
    }

    #[must_use]
    pub const fn rpm_limit(&self) -> Option<i64> {
        match *self {
            Self::New(request) => request.team.rpm_limit,
            Self::Update(request) => request.fields.rpm_limit,
        }
    }

    #[must_use]
    pub const fn metadata(&self) -> Option<&'request BTreeMap<String, JsonData>> {
        match *self {
            Self::New(request) => request.team.metadata.as_ref(),
            Self::Update(request) => request.fields.metadata.as_ref(),
        }
    }

    #[must_use]
    pub const fn model_rpm_limit(&self) -> Option<&'request BTreeMap<String, i64>> {
        match *self {
            Self::New(request) => request.extra.model_rpm_limit.as_ref(),
            Self::Update(request) => request.fields.model_rpm_limit.as_ref(),
        }
    }

    #[must_use]
    pub const fn model_tpm_limit(&self) -> Option<&'request BTreeMap<String, i64>> {
        match *self {
            Self::New(request) => request.extra.model_tpm_limit.as_ref(),
            Self::Update(request) => request.fields.model_tpm_limit.as_ref(),
        }
    }

    // UpdateTeamRequest declares no rpm_limit_type or tpm_limit_type, so an update reads them from metadata
    #[must_use]
    pub const fn rpm_limit_type(&self) -> Option<ThroughputLimitType> {
        match *self {
            Self::New(request) => request.extra.rpm_limit_type,
            Self::Update(_) => None,
        }
    }

    #[must_use]
    pub const fn tpm_limit_type(&self) -> Option<ThroughputLimitType> {
        match *self {
            Self::New(request) => request.extra.tpm_limit_type,
            Self::Update(_) => None,
        }
    }
}

use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_config::sections::key_limit_policy::UpperboundKeyGenerateParams;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::key_repository::decode::TokenDecodeError;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::key_fields::request::GenerateKeyRequest;
use betterlite_server::key_fields::update::UpdateKeyRequest;
use betterlite_server::key_limit_policy::checks::{
    check_org_key_limits, check_project_key_limits, check_team_key_limits, validate_org_assignment,
};
use betterlite_server::key_limit_policy::error::KeyLimitError;
use betterlite_server::key_limit_policy::reads::{
    KeyLimitReadError, KeyLimitReads, KeyOwner, member_ids,
};
use betterlite_server::key_limit_policy::request::KeyLimitRequest;
use betterlite_server::key_limit_policy::upperbound::{TimeContext, enforce_upperbound};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::runtime::Builder;

// the team and org rows outweigh every other variant, so the input boxes them as the row enums do
#[derive(Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
pub(crate) enum Call {
    Upperbound {
        request: RequestInput,
        params: Option<UpperboundKeyGenerateParams>,
        fill_defaults: bool,
    },
    Team {
        request: RequestInput,
        team: Box<LiteLLMTeamTableCachedObj>,
        keys: Vec<JsonData>,
    },
    Project {
        request: RequestInput,
        project_id: String,
        project: Option<JsonData>,
    },
    Organization {
        request: RequestInput,
        organization: Box<OrganizationTable>,
        keys: Vec<JsonData>,
    },
    Assignment {
        caller_user_id: Option<String>,
        organization_id: String,
        users: Vec<JsonData>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub(crate) enum RequestInput {
    Generate(JsonData),
    Update(JsonData),
}

impl RequestInput {
    fn parse(&self) -> Result<KeyLimitRequest, Box<HelperError>> {
        match self {
            Self::Generate(body) => GenerateKeyRequest::read(&TypedValue { json: body.clone() })
                .map(|request| KeyLimitRequest::from(&request)),
            Self::Update(body) => UpdateKeyRequest::read(&TypedValue { json: body.clone() })
                .map(|request| KeyLimitRequest::from(&request)),
        }
        .or_value_error()
    }
}

// each call reads one table, so the case's rows stand for that table and match on the where field
struct FakeReads<'rows> {
    rows: &'rows [JsonData],
}

#[async_trait]
impl KeyLimitReads for FakeReads<'_> {
    async fn keys(
        &self,
        owner: KeyOwner<'_>,
    ) -> Result<Vec<LiteLLMVerificationToken>, KeyLimitReadError> {
        let name = owner.column().name();
        let found = self.rows.iter().filter(|row| holds(row, name, owner.id()));
        found.map(stored).collect()
    }

    async fn project(&self, project_id: &str) -> Result<Option<CachedProject>, KeyLimitReadError> {
        let found = self
            .rows
            .iter()
            .find(|row| holds(row, "project_id", project_id));
        found.map(stored).transpose()
    }

    async fn member_organizations(
        &self,
        user_id: &str,
    ) -> Result<BTreeSet<String>, KeyLimitReadError> {
        let found = self.rows.iter().find(|row| holds(row, "user_id", user_id));
        found.map_or_else(|| Ok(BTreeSet::new()), member_ids)
    }
}

fn holds(row: &JsonData, field: &str, value: &str) -> bool {
    matches!(
        row,
        JsonData::Object(fields)
            if matches!(fields.get(field), Some(JsonData::String(id)) if id == value)
    )
}

fn stored<T>(row: &JsonData) -> Result<T, KeyLimitReadError>
where
    T: DeserializeOwned,
{
    serde_json::to_value(row)
        .and_then(serde_json::from_value)
        .map_err(|source| KeyLimitReadError::Decode(TokenDecodeError::Model { source }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?
            .block_on(evaluate(input))
    }
}

async fn evaluate(call: &Call) -> HelperResult {
    let checked = match call {
        Call::Upperbound {
            request,
            params,
            fill_defaults,
        } => return upperbound(&request.parse()?, params.as_ref(), *fill_defaults),
        Call::Team {
            request,
            team,
            keys,
        } => check_team_key_limits(team, &request.parse()?, &FakeReads { rows: keys }).await,
        Call::Project {
            request,
            project_id,
            project,
        } => {
            let rows = project.as_slice();
            check_project_key_limits(project_id, &request.parse()?, &FakeReads { rows }).await
        }
        Call::Organization {
            request,
            organization,
            keys,
        } => check_org_key_limits(organization, &request.parse()?, &FakeReads { rows: keys }).await,
        Call::Assignment {
            caller_user_id,
            organization_id,
            users,
        } => {
            let caller = caller_user_id.as_deref();
            validate_org_assignment(caller, organization_id, &FakeReads { rows: users }).await
        }
    };
    answered(checked.map(|()| Value::Null))
}

fn upperbound(
    request: &KeyLimitRequest,
    params: Option<&UpperboundKeyGenerateParams>,
    fill: bool,
) -> HelperResult {
    let database = TimeZoneDatabase::from_env();
    let time = TimeContext {
        now: DateTime::<Utc>::UNIX_EPOCH,
        timezone: None,
        database: &database,
    };
    let enforced = enforce_upperbound(request, params, fill, &time);
    answered(enforced.map(|values| json!({ "values": values })))
}

fn answered(outcome: Result<Value, KeyLimitError>) -> HelperResult {
    match outcome {
        Ok(answer) => Ok(answer),
        Err(KeyLimitError::Rejected(rejection)) => {
            Ok(json!({"status": rejection.status(), "detail": rejection.detail()}))
        }
        Err(error) => Err(error).or_value_error(),
    }
}

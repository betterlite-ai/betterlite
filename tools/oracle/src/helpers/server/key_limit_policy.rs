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
use betterlite_server::key_limit_policy::error::{KeyLimitError, Rejection};
use betterlite_server::key_limit_policy::reads::{KeyLimitReadError, KeyLimitReads, KeyOwner};
use betterlite_server::key_limit_policy::request::KeyLimitRequest;
use betterlite_server::key_limit_policy::upperbound::{TimeContext, enforce_upperbound};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::runtime::Builder;

// one production entry point with the rows and request the case supplies
#[derive(Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
enum Call {
    Upperbound {
        request: RequestInput,
        params: Option<UpperboundKeyGenerateParams>,
        fill_defaults: bool,
    },
    Team {
        request: RequestInput,
        team: JsonData,
        keys: Vec<JsonData>,
    },
    Project {
        request: RequestInput,
        project_id: String,
        project: Option<JsonData>,
    },
    Organization {
        request: RequestInput,
        organization: JsonData,
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
enum RequestInput {
    Generate(JsonData),
    Update(JsonData),
}

enum ParsedRequest {
    Generate(GenerateKeyRequest),
    Update(UpdateKeyRequest),
}

impl RequestInput {
    fn parse(&self) -> Result<ParsedRequest, Box<HelperError>> {
        match self {
            Self::Generate(body) => {
                GenerateKeyRequest::read(&typed(body)).map(ParsedRequest::Generate)
            }
            Self::Update(body) => UpdateKeyRequest::read(&typed(body)).map(ParsedRequest::Update),
        }
        .or_value_error()
    }
}

impl<'request> From<&'request ParsedRequest> for KeyLimitRequest<'request> {
    fn from(parsed: &'request ParsedRequest) -> Self {
        match parsed {
            ParsedRequest::Generate(request) => Self::Generate(request),
            ParsedRequest::Update(request) => Self::Update(request),
        }
    }
}

fn typed(body: &JsonData) -> TypedValue {
    TypedValue { json: body.clone() }
}

// the only reads the checks perform: key rows by owner, a project row, a caller's memberships
struct FakeReads {
    keys: Vec<JsonData>,
    projects: Vec<JsonData>,
    users: Vec<JsonData>,
}

impl FakeReads {
    fn with_keys(keys: &[JsonData]) -> Self {
        Self {
            keys: keys.to_vec(),
            projects: Vec::new(),
            users: Vec::new(),
        }
    }

    fn with_project(project: Option<&JsonData>) -> Self {
        Self {
            keys: Vec::new(),
            projects: project.cloned().into_iter().collect(),
            users: Vec::new(),
        }
    }

    fn with_users(users: &[JsonData]) -> Self {
        Self {
            keys: Vec::new(),
            projects: Vec::new(),
            users: users.to_vec(),
        }
    }
}

#[async_trait]
impl KeyLimitReads for FakeReads {
    async fn keys(
        &self,
        owner: KeyOwner<'_>,
    ) -> Result<Vec<LiteLLMVerificationToken>, KeyLimitReadError> {
        select(&self.keys, owner.column().name(), owner.id())
    }

    async fn project(&self, project_id: &str) -> Result<Option<CachedProject>, KeyLimitReadError> {
        Ok(select(&self.projects, "project_id", project_id)?.pop())
    }

    async fn member_organizations(
        &self,
        user_id: &str,
    ) -> Result<BTreeSet<String>, KeyLimitReadError> {
        let mut memberships = BTreeSet::new();
        for row in select(&self.users, "user_id", user_id)? {
            let JsonData::Object(fields) = row else {
                continue;
            };
            let Some(JsonData::Array(entries)) = fields.get("organization_memberships") else {
                continue;
            };
            for entry in entries {
                if let JsonData::Object(membership) = entry
                    && let Some(JsonData::String(id)) = membership.get("organization_id")
                {
                    memberships.insert(id.clone());
                }
            }
        }
        Ok(memberships)
    }
}

fn select<T>(rows: &[JsonData], field: &str, value: &str) -> Result<Vec<T>, KeyLimitReadError>
where
    T: DeserializeOwned,
{
    rows.iter()
        .filter(|row| {
            matches!(
                row,
                JsonData::Object(fields)
                    if matches!(fields.get(field), Some(JsonData::String(id)) if id == value)
            )
        })
        .map(stored)
        .collect()
}

fn stored<T>(row: &JsonData) -> Result<T, KeyLimitReadError>
where
    T: DeserializeOwned,
{
    serde_json::to_value(row)
        .and_then(serde_json::from_value)
        .map_err(|source| KeyLimitReadError::Decode(TokenDecodeError::Model { source }))
}

fn decode<T>(value: &JsonData) -> Result<T, Box<HelperError>>
where
    T: DeserializeOwned,
{
    stored(value).or_value_error()
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
    match call {
        Call::Upperbound {
            request,
            params,
            fill_defaults,
        } => upperbound(request, params.as_ref(), *fill_defaults),
        Call::Team {
            request,
            team,
            keys,
        } => {
            let parsed = request.parse()?;
            let table: LiteLLMTeamTableCachedObj = decode(team)?;
            let limits = KeyLimitRequest::from(&parsed);
            judged(check_team_key_limits(&table, &limits, &FakeReads::with_keys(keys)).await)
        }
        Call::Project {
            request,
            project_id,
            project,
        } => {
            let parsed = request.parse()?;
            let limits = KeyLimitRequest::from(&parsed);
            judged(
                check_project_key_limits(
                    project_id,
                    &limits,
                    &FakeReads::with_project(project.as_ref()),
                )
                .await,
            )
        }
        Call::Organization {
            request,
            organization,
            keys,
        } => {
            let parsed = request.parse()?;
            let table: OrganizationTable = decode(organization)?;
            let limits = KeyLimitRequest::from(&parsed);
            judged(check_org_key_limits(&table, &limits, &FakeReads::with_keys(keys)).await)
        }
        Call::Assignment {
            caller_user_id,
            organization_id,
            users,
        } => judged(
            validate_org_assignment(
                caller_user_id.as_deref(),
                organization_id,
                &FakeReads::with_users(users),
            )
            .await,
        ),
    }
}

fn upperbound(
    request: &RequestInput,
    params: Option<&UpperboundKeyGenerateParams>,
    fill_defaults: bool,
) -> HelperResult {
    let parsed = request.parse()?;
    let database = TimeZoneDatabase::from_env();
    let time = TimeContext {
        now: DateTime::<Utc>::UNIX_EPOCH,
        timezone: None,
        database: &database,
    };
    match enforce_upperbound(
        &KeyLimitRequest::from(&parsed),
        params,
        fill_defaults,
        &time,
    ) {
        Ok(values) => Ok(json!({
            "values": serde_json::to_value(values).or_value_error()?,
        })),
        Err(KeyLimitError::Rejected(rejection)) => Ok(rejected(&rejection)),
        Err(error) => Err(error).or_value_error(),
    }
}

fn judged(result: Result<(), KeyLimitError>) -> HelperResult {
    match result {
        Ok(()) => Ok(Value::Null),
        Err(KeyLimitError::Rejected(rejection)) => Ok(rejected(&rejection)),
        Err(error) => Err(error).or_value_error(),
    }
}

fn rejected(rejection: &Rejection) -> Value {
    json!({"status": rejection.status(), "detail": rejection.detail()})
}

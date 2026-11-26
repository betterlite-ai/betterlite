mod context;
mod fixture_support;
mod policy;

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

pub(crate) struct Op;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Guardrail,
    Tools,
    TeamFromToken,
    Vouch,
    OrgInherit,
    SafeFetch,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Lookup {
    TeamNotFound,
    Degraded,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Proxy,
    TeamNotFound,
    NoDatabase,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) route: String,
    #[serde(default)]
    pub(crate) body: JsonObject,
    pub(crate) token: Option<Value>,
    pub(crate) team: Option<Value>,
    pub(crate) org: Option<Value>,
    pub(crate) lookup: Option<Lookup>,
    pub(crate) failure: Option<Failure>,
    #[serde(default)]
    pub(crate) database: bool,
    #[serde(default)]
    pub(crate) outage: bool,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Guardrail => policy::guardrail(input),
            Function::Tools => policy::tools(input),
            Function::TeamFromToken => context::team_from_token(input),
            Function::Vouch => context::vouch(input),
            Function::OrgInherit => context::org_inherit(input),
            Function::SafeFetch => context::safe_fetch_outcome(input),
        }
    }
}

pub(super) fn team_obj(
    value: Option<&Value>,
) -> Result<Option<LiteLLMTeamTable>, Box<HelperError>> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let row = serde_json::from_value(raw.clone()).or_value_error()?;
    Ok(Some(row))
}

pub(super) fn token_in(value: Option<&Value>) -> Result<Option<UserAPIKeyAuth>, Box<HelperError>> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let token = token_obj(raw)?;
    Ok(Some(token))
}

pub(super) fn required_token(value: Option<&Value>) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let Some(raw) = value else {
        return Err(invalid("the call needs a token"));
    };
    token_obj(raw)
}

fn token_obj(value: &Value) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let JsonData::Object(fields) = serde_json::from_value(value.clone()).or_value_error()? else {
        return Err(invalid("the token must be an object"));
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    let built = Builder::new_current_thread().enable_all().build();
    let runtime = built.or_value_error()?;
    Ok(runtime)
}

pub(super) fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

pub(super) fn proxy_denial() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "denied".to_owned(),
        error_type: "auth_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(403),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

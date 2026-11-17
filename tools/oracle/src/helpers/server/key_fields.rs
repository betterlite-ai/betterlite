use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_fields::base::{GenerateRequestBase, KeyRequestBase};
use betterlite_server::key_fields::broadcast::KeyUpdateFields;
use betterlite_server::key_fields::bulk::BulkUpdateTeamKeysRequest;
use betterlite_server::key_fields::decode::{TypedValue, decode_key_request};
use betterlite_server::key_fields::policy::CustomKeyPolicyRequest;
use betterlite_server::key_fields::regenerate::RegenerateKeyRequest;
use betterlite_server::key_fields::request::GenerateKeyRequest;
use betterlite_server::key_fields::response::GenerateKeyResponse;
use betterlite_server::key_fields::shared::{hash_token_if_needed, with_settings_updated_at};
use betterlite_server::key_fields::update::UpdateKeyRequest;
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Base,
    Key,
    Generate,
    Update,
    Regenerate,
    Response,
    Broadcast,
    TeamKeys,
    Policy,
    Settings,
    Token,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    #[serde(default)]
    body: Option<JsonData>,
    #[serde(default)]
    token: Option<JsonData>,
    #[serde(default)]
    now: Option<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(serde::Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        serde_json::to_value(input.calls.iter().map(run_call).collect::<Vec<_>>()).or_value_error()
    }
}

fn run_call(call: &Call) -> Outcome {
    match evaluate(call) {
        Ok(value) => Outcome {
            ok: true,
            value: Some(value),
        },
        Err(_) => Outcome {
            ok: false,
            value: None,
        },
    }
}

fn evaluate(call: &Call) -> Result<serde_json::Value, Box<HelperError>> {
    match call.function {
        Function::Base => base(call),
        Function::Key => key(call),
        Function::Generate => generate(call),
        Function::Update => update(call),
        Function::Regenerate => regenerate(call),
        Function::Response => response(call),
        Function::Broadcast => broadcast(call),
        Function::TeamKeys => team_keys(call),
        Function::Policy => policy(call),
        Function::Settings => settings(call),
        Function::Token => token(call),
    }
}

fn value_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_owned(),
    })
}

fn document(call: &Call) -> Result<TypedValue, Box<HelperError>> {
    let Some(body) = &call.body else {
        return Err(value_error("the body must be an object"));
    };
    let bytes = serde_json::to_vec(body).or_value_error()?;
    decode_key_request(&bytes).or_value_error()
}

macro_rules! projection_call {
    ($name:ident, $request:ty) => {
        fn $name(call: &Call) -> Result<serde_json::Value, Box<HelperError>> {
            let request = <$request>::read(&document(call)?).or_value_error()?;
            let projection = request.project().or_value_error()?;
            serde_json::to_value(projection).or_value_error()
        }
    };
}

projection_call!(base, GenerateRequestBase);
projection_call!(key, KeyRequestBase);
projection_call!(generate, GenerateKeyRequest);
projection_call!(update, UpdateKeyRequest);
projection_call!(regenerate, RegenerateKeyRequest);
projection_call!(response, GenerateKeyResponse);
projection_call!(broadcast, KeyUpdateFields);

fn team_keys(call: &Call) -> Result<serde_json::Value, Box<HelperError>> {
    let request = BulkUpdateTeamKeysRequest::read(&document(call)?).or_value_error()?;
    let resolved = request.resolved_object().or_value_error()?;
    serde_json::to_value(resolved).or_value_error()
}

fn policy(call: &Call) -> Result<serde_json::Value, Box<HelperError>> {
    let record = CustomKeyPolicyRequest::read(&document(call)?).or_value_error()?;
    let projected = record.project().or_value_error()?;
    serde_json::to_value(projected).or_value_error()
}

fn token(call: &Call) -> Result<serde_json::Value, Box<HelperError>> {
    let Some(JsonData::String(token)) = &call.token else {
        return Err(value_error("the token must be a string"));
    };
    serde_json::to_value(hash_token_if_needed(token)).or_value_error()
}

fn settings(call: &Call) -> Result<serde_json::Value, Box<HelperError>> {
    let Some(JsonData::Object(body)) = &call.body else {
        return Err(value_error("the body must be an object"));
    };
    let Some(JsonData::String(now_text)) = &call.now else {
        return Err(value_error("the now must be a string"));
    };
    let now = DateTime::parse_from_rfc3339(now_text)
        .or_value_error()?
        .with_timezone(&Utc);
    serde_json::to_value(with_settings_updated_at(body, now)).or_value_error()
}

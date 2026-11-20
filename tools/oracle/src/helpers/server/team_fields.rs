use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::team_fields::{
    Raw as RawTeamFields, Settings as TeamFieldsSettings, ValidationTimeout,
    parse_team_metadata_schema,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::team_fields::metadata::{
    MetadataRequest, TeamMetadataOperation, TeamMetadataRequester, TeamMetadataValidationPayload,
};
use betterlite_server::team_fields::rows::{
    CallerRole, GeneralSettingsMap, LogValue, TeamEditAccess, caller_edit_access, sanitize_for_log,
};
use betterlite_server::team_fields::validation::{
    TeamMetadataValidator, TeamMetadataValidatorRegistry, ValidatorFailure,
    validate_team_metadata_if_configured,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

// The Python helper's sleep validator waits this long, so a deadline below it times the callback out.
const SLEEP_SECONDS: Duration = Duration::from_millis(200);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequesterInput {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_email: Option<String>,
    #[serde(default)]
    user_role: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Schema {
        schema: JsonValue,
    },
    Registry {
        steps: Vec<String>,
    },
    Validate {
        general_settings: BTreeMap<String, JsonValue>,
        validator: String,
        operation: String,
        #[serde(default)]
        metadata: Option<JsonValue>,
        #[serde(default)]
        existing_metadata: Option<JsonValue>,
        #[serde(default)]
        team_id: Option<String>,
        #[serde(default)]
        team_alias: Option<String>,
        requester: RequesterInput,
    },
    Sanitize {
        value: String,
    },
    EditAccess {
        #[serde(default)]
        role: Option<String>,
        general_settings: BTreeMap<String, JsonValue>,
    },
}

struct ValidateCall<'input> {
    general_settings: &'input BTreeMap<String, JsonValue>,
    validator: &'input str,
    operation: &'input str,
    metadata: Option<&'input JsonValue>,
    existing_metadata: Option<&'input JsonValue>,
    team_id: Option<&'input str>,
    team_alias: Option<&'input str>,
    requester: &'input RequesterInput,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Schema { schema } => Ok(schema_answer(schema)),
            Input::Registry { steps } => registry_answer(steps),
            Input::Validate {
                general_settings,
                validator,
                operation,
                metadata,
                existing_metadata,
                team_id,
                team_alias,
                requester,
            } => validate_answer(&ValidateCall {
                general_settings,
                validator,
                operation,
                metadata: metadata.as_ref(),
                existing_metadata: existing_metadata.as_ref(),
                team_id: team_id.as_deref(),
                team_alias: team_alias.as_deref(),
                requester,
            }),
            Input::Sanitize { value } => Ok(json!({
                "text": sanitize_for_log(LogValue::Display(value)),
            })),
            Input::EditAccess {
                role,
                general_settings,
            } => edit_access_answer(role.as_deref(), general_settings),
        }
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn schema_answer(schema: &JsonValue) -> Value {
    match parse_team_metadata_schema(Some(schema)) {
        Ok(fields) => json!({
            "fields": fields
                .iter()
                .map(|field| json!({"key": field.key, "label": field.label}))
                .collect::<Vec<_>>(),
        }),
        Err(_) => json!({"failed": true}),
    }
}

#[derive(Clone, Copy)]
enum Behaviour {
    Accept,
    AcceptUnrecorded,
    Reject,
    RejectEmpty,
    Fail,
    MalformedType,
    MalformedObject,
    Sleep,
}

struct FakeValidator {
    asynchronous: bool,
    behaviour: Behaviour,
    seen: Mutex<Option<Value>>,
}

impl FakeValidator {
    const fn new(behaviour: Behaviour, asynchronous: bool) -> Self {
        Self {
            asynchronous,
            behaviour,
            seen: Mutex::new(None),
        }
    }

    fn recorded(&self) -> Option<Value> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn data(value: Value) -> Result<JsonData, ValidatorFailure> {
    serde_json::from_value(value).map_err(|_error| ValidatorFailure::NotJson)
}

#[async_trait]
impl TeamMetadataValidator for FakeValidator {
    fn is_async(&self) -> bool {
        self.asynchronous
    }

    async fn invoke(
        &self,
        payload: &TeamMetadataValidationPayload,
        timeout: ValidationTimeout,
    ) -> Result<JsonData, ValidatorFailure> {
        if !matches!(self.behaviour, Behaviour::AcceptUnrecorded) {
            let captured =
                serde_json::to_value(payload).map_err(|_error| ValidatorFailure::NotJson)?;
            *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(captured);
        }
        match self.behaviour {
            Behaviour::Accept | Behaviour::AcceptUnrecorded => data(json!({"valid": true})),
            Behaviour::Reject => {
                data(json!({"valid": false, "error_message": "alias must be set"}))
            }
            Behaviour::RejectEmpty => data(json!({"valid": false, "error_message": ""})),
            Behaviour::Fail => Err(ValidatorFailure::NotJson),
            Behaviour::MalformedType => data(json!({"valid": "yes"})),
            Behaviour::MalformedObject => Ok(JsonData::String("ok".to_owned())),
            Behaviour::Sleep => match timeout {
                ValidationTimeout::Finite(after) if after < SLEEP_SECONDS => {
                    Err(ValidatorFailure::TimedOut { after })
                }
                ValidationTimeout::Finite(_) | ValidationTimeout::Unlimited => {
                    data(json!({"valid": true}))
                }
            },
        }
    }
}

fn validator_for(name: &str) -> Result<Option<(Behaviour, bool)>, Box<HelperError>> {
    let found = match name {
        "absent" => return Ok(None),
        "function" => (Behaviour::AcceptUnrecorded, true),
        "accept" => (Behaviour::Accept, true),
        "sync" => (Behaviour::Accept, false),
        "reject" => (Behaviour::Reject, true),
        "reject_empty" => (Behaviour::RejectEmpty, true),
        "raises" => (Behaviour::Fail, true),
        "malformed_type" => (Behaviour::MalformedType, true),
        "malformed_object" => (Behaviour::MalformedObject, true),
        "sleep" => (Behaviour::Sleep, true),
        other => return Err(invalid(format!("unknown validator {other}"))),
    };
    Ok(Some(found))
}

fn settings_from(
    general_settings: &BTreeMap<String, JsonValue>,
) -> Result<TeamFieldsSettings, Box<HelperError>> {
    let raw: RawTeamFields =
        serde_json::from_value(json!({"general_settings": general_settings})).or_value_error()?;
    raw.convert(&BTreeMap::<String, String>::new())
        .or_value_error()
}

fn json_data(value: &JsonValue) -> Result<JsonData, Box<HelperError>> {
    serde_json::from_value(value.clone()).or_value_error()
}

fn validate_answer(call: &ValidateCall<'_>) -> HelperResult {
    let settings = settings_from(call.general_settings)?;
    let found = validator_for(call.validator)?;
    let registry = TeamMetadataValidatorRegistry::default();
    let fake = found
        .map(|(behaviour, asynchronous)| Arc::new(FakeValidator::new(behaviour, asynchronous)));
    if let Some(installed) = &fake {
        registry.set(Some(Arc::clone(installed) as Arc<dyn TeamMetadataValidator>));
    }
    let operation: TeamMetadataOperation =
        serde_json::from_value(json!(call.operation)).or_value_error()?;
    let metadata = match call.metadata {
        Some(value) => json_data(value)?,
        None => JsonData::Null,
    };
    let existing = call.existing_metadata.map(json_data).transpose()?;
    let request = MetadataRequest {
        operation,
        metadata: &metadata,
        existing_metadata: existing.as_ref(),
        team_id: call.team_id,
        team_alias: call.team_alias,
        requester: TeamMetadataRequester {
            user_id: call.requester.user_id.clone(),
            user_email: call.requester.user_email.clone(),
            user_role: call.requester.user_role.clone(),
        },
    };
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    let outcome = runtime.block_on(validate_team_metadata_if_configured(
        request, &registry, &settings,
    ));
    let payload = fake.as_ref().and_then(|fake| fake.recorded());
    match outcome {
        Ok(()) => Ok(json!({"status": null, "error": null, "payload": payload})),
        Err(error) => Ok(json!({
            "status": error.status(),
            "error": error.message(),
            "payload": payload,
        })),
    }
}

fn registry_answer(steps: &[String]) -> HelperResult {
    let registry = TeamMetadataValidatorRegistry::default();
    let first: Arc<dyn TeamMetadataValidator> =
        Arc::new(FakeValidator::new(Behaviour::Accept, true));
    let second: Arc<dyn TeamMetadataValidator> =
        Arc::new(FakeValidator::new(Behaviour::Reject, true));
    let mut seen = Vec::new();
    for step in steps {
        match step.as_str() {
            "get" => {
                let current = registry.get();
                let name = match current {
                    None => "none",
                    Some(found) if Arc::ptr_eq(&found, &first) => "a",
                    Some(_) => "b",
                };
                seen.push(name);
            }
            "set_a" => registry.set(Some(Arc::clone(&first))),
            "set_b" => registry.set(Some(Arc::clone(&second))),
            "clear" => registry.set(None),
            other => return Err(invalid(format!("unknown registry step {other}"))),
        }
    }
    Ok(json!({"seen": seen}))
}

fn edit_access_answer(
    role: Option<&str>,
    general_settings: &BTreeMap<String, JsonValue>,
) -> HelperResult {
    let caller = match role {
        None => None,
        Some("proxy_admin") => Some(CallerRole::ProxyAdmin),
        Some("org_admin") => Some(CallerRole::OrgAdmin),
        Some("team_admin") => Some(CallerRole::TeamAdmin),
        Some(other) => return Err(invalid(format!("unknown role {other}"))),
    };
    let settings = GeneralSettingsMap {
        entries: general_settings.clone(),
    };
    let (kind, fields) = match caller_edit_access(caller, &settings) {
        TeamEditAccess::Unrestricted => ("unrestricted", None),
        TeamEditAccess::TeamAdmin { editable_fields } => ("team_admin", Some(editable_fields)),
        TeamEditAccess::TeamAdminDisabled => ("team_admin_disabled", None),
        TeamEditAccess::None => ("none", None),
    };
    Ok(json!({"kind": kind, "editable_fields": fields}))
}

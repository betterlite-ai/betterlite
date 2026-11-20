use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::team_fields::{
    Raw as RawTeamFields, Settings as TeamFieldsSettings, ValidationTimeout,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::team_fields::metadata::{
    MetadataRequest, TeamMetadataOperation, TeamMetadataRequester, TeamMetadataValidationPayload,
};
use betterlite_server::team_fields::python_validator::ValidatorFailure;
use betterlite_server::team_fields::validation::{
    TeamMetadataValidator,
    TeamMetadataValidatorRegistry,
    // ast-grep-ignore: no-long-names - LiteLLM names this validate_team_metadata_if_configured
    validate_team_metadata_if_configured,
};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::{ValidateCall, invalid};

// The Python helper's sleep validator waits this long, so a deadline below it times the callback out.
const SLEEP_SECONDS: Duration = Duration::from_millis(200);

#[derive(Clone, Copy)]
pub(super) enum Behaviour {
    Accept,
    AcceptUnrecorded,
    Reject,
    RejectEmpty,
    Fail,
    MalformedType,
    MalformedObject,
    Sleep,
}

pub(super) struct FakeValidator {
    asynchronous: bool,
    behaviour: Behaviour,
    seen: Mutex<Option<Value>>,
}

impl FakeValidator {
    pub(super) const fn new(behaviour: Behaviour, asynchronous: bool) -> Self {
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
    match serde_json::from_value(value) {
        Ok(data) => Ok(data),
        Err(_error) => Err(ValidatorFailure::NotJson),
    }
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
    match serde_json::from_value(value.clone()) {
        Ok(data) => Ok(data),
        Err(error) => Err(error).or_value_error(),
    }
}

pub(super) fn validate_answer(call: &ValidateCall<'_>) -> HelperResult {
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
    // ast-grep-ignore: no-long-names - LiteLLM names this validate_team_metadata_if_configured
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

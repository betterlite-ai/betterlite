use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_auth::auth_user_lookup::cadence::{Cadence, CadenceClock};
use betterlite_auth::auth_user_lookup::error::{UserLookupError, user_read_failure};
use betterlite_auth::auth_user_lookup::roles::RolePermissions;
use betterlite_auth::auth_user_lookup::store::UserStoreError;
use betterlite_config::sections::auth_user_lookup::{RbacRole, RoleBasedPermissions};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

// the pure pieces of get_user_object: the cadence index, role selection and the read-failure classifier
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Cadence {
        max_size: usize,
        times_ms: Vec<u64>,
        operations: Vec<CadenceOperation>,
    },
    RoleSelection {
        role: RbacRole,
        #[serde(default)]
        role_permissions: Option<JsonData>,
    },
    RoleValidation {
        role_permissions: JsonData,
    },
    ReadErrorClassification {
        errors: Vec<ErrorCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum CadenceOperation {
    ShouldCheck {
        key: String,
        expiry: i64,
    },
    Update {
        key: String,
        value: Option<JsonData>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ErrorCase {
    Message { message: String },
    Io { message: String },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Cadence {
                max_size,
                times_ms,
                operations,
            } => Ok(cadence_report(*max_size, times_ms, operations)),
            Input::RoleSelection {
                role,
                role_permissions,
            } => Ok(role_report(*role, role_permissions.as_ref())),
            Input::RoleValidation { role_permissions } => Ok(validation_report(role_permissions)),
            Input::ReadErrorClassification { errors } => Ok(classification_report(errors)),
        }
    }
}

// time.time() under test control: each sampled call takes the next scripted millisecond
struct ScriptedClock {
    state: Mutex<ScriptedTimes>,
}

struct ScriptedTimes {
    remaining: VecDeque<u64>,
    last: Duration,
}

impl ScriptedClock {
    fn new(times_ms: &[u64]) -> Self {
        Self {
            state: Mutex::new(ScriptedTimes {
                remaining: times_ms.iter().copied().collect(),
                last: Duration::ZERO,
            }),
        }
    }
}

impl CadenceClock for ScriptedClock {
    fn now(&self) -> Duration {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(millis) = state.remaining.pop_front() {
            state.last = Duration::from_millis(millis);
        }
        state.last
    }
}

fn cadence_report(max_size: usize, times_ms: &[u64], operations: &[CadenceOperation]) -> Value {
    let cadence = Cadence::new(max_size, Arc::new(ScriptedClock::new(times_ms)));
    let mut decisions = Vec::new();
    for operation in operations {
        match operation {
            CadenceOperation::ShouldCheck { key, expiry } => {
                decisions.push(cadence.should_check_db(key, *expiry));
            }
            CadenceOperation::Update { key, value } => cadence.update(key, value.clone()),
        }
    }
    json!({"decisions": decisions})
}

fn role_report(role: RbacRole, configured: Option<&JsonData>) -> Value {
    let absent = JsonData::Null;
    let configured = configured.unwrap_or(&absent);
    match RolePermissions::from_json(configured) {
        Ok(permissions) => json!({
            "valid": true,
            "models": permissions.models(role),
            "routes": permissions.routes(role),
        }),
        Err(_error) => json!({"valid": false}),
    }
}

fn validation_report(configured: &JsonData) -> Value {
    let parsed = serde_json::to_value(configured)
        .and_then(serde_json::from_value::<Vec<RoleBasedPermissions>>);
    match parsed {
        Ok(entries) => json!({"valid": true, "entries": entries}),
        Err(_error) => json!({"valid": false}),
    }
}

fn classification_report(errors: &[ErrorCase]) -> Value {
    let mut reports = Vec::with_capacity(errors.len());
    for error in errors {
        reports.push(classification(error));
    }
    json!({"errors": reports})
}

fn classification(error: &ErrorCase) -> Value {
    let failure = match error {
        ErrorCase::Message { message } => {
            UserStoreError::Other(Box::new(NamedFailure(message.clone())))
        }
        ErrorCase::Io { message } => {
            UserStoreError::Other(Box::new(std::io::Error::other(message.clone())))
        }
    };
    match user_read_failure("u1", Box::new(failure)) {
        UserLookupError::Unavailable(cause) => {
            json!({"kind": "unavailable", "message": cause.to_string()})
        }
        UserLookupError::UserMissing(failure) => {
            json!({"kind": "missing", "message": failure.to_string()})
        }
        UserLookupError::NotFound(error) => {
            json!({"kind": "not_found", "message": error.to_string()})
        }
        UserLookupError::NoDatabase => json!({"kind": "no_database", "message": ""}),
        UserLookupError::PreLookup(error) => {
            json!({"kind": "pre_lookup", "message": error.to_string()})
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct NamedFailure(String);

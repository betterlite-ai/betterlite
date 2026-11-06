use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::management_guards::error::ManagementGuardError;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct Outcome {
    ok: bool,
    status: Option<u16>,
    detail: Option<String>,
    value: JsonData,
}

pub(super) enum GuardFailure {
    Rejected {
        error: ManagementGuardError,
        value: JsonData,
    },
    InvalidInput(&'static str),
}

impl From<ManagementGuardError> for GuardFailure {
    fn from(error: ManagementGuardError) -> Self {
        Self::Rejected {
            error,
            value: JsonData::Null,
        }
    }
}

pub(super) fn into_outcome(result: Result<JsonData, GuardFailure>) -> Outcome {
    match result {
        Ok(value) => Outcome {
            ok: true,
            status: None,
            detail: None,
            value,
        },
        Err(GuardFailure::Rejected { error, value }) => Outcome {
            ok: false,
            status: error.status_code().map(|status| status.as_u16()),
            detail: Some(error.to_string()),
            value,
        },
        Err(GuardFailure::InvalidInput(message)) => Outcome {
            ok: false,
            status: None,
            detail: Some(message.to_owned()),
            value: JsonData::Null,
        },
    }
}

pub(super) fn guarded(result: Result<(), ManagementGuardError>) -> Result<JsonData, GuardFailure> {
    result.map_err(GuardFailure::from)?;
    Ok(JsonData::Null)
}

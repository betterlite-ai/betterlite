use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::permission::KeyPermissionLookup;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use serde_json::json;

use super::PermissionInput;

pub(super) struct ScriptedPermissions {
    answers: Mutex<VecDeque<Result<Option<LiteLLMObjectPermissionTable>, PermissionError>>>,
    modes: Mutex<Vec<bool>>,
}

impl ScriptedPermissions {
    pub(super) fn call_modes(&self) -> Vec<bool> {
        self.modes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl KeyPermissionLookup for ScriptedPermissions {
    async fn permission(
        &self,
        _object_permission_id: &str,
        check_db_only: bool,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        self.modes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(check_db_only);
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(None))
    }
}

pub(super) fn permission_fixture(
    input: Option<&PermissionInput>,
) -> Result<Arc<ScriptedPermissions>, Box<HelperError>> {
    let Some(fields) = input else {
        return Ok(Arc::new(ScriptedPermissions {
            answers: Mutex::new(VecDeque::from([Ok(None)])),
            modes: Mutex::new(Vec::new()),
        }));
    };
    let answer = if fields.fail {
        Err(PermissionError::Lookup(DbLookupError::Database(DbError {
            error_type: ErrorType::Connect,
            message: "connection closed".to_owned(),
            code: None,
            meta_code: None,
            traceback_modules: Vec::new(),
            cause: None,
        })))
    } else {
        let table: LiteLLMObjectPermissionTable = serde_json::from_value(json!({
            "object_permission_id": fields.id,
            "models": ["gpt-4"],
        }))
        .or_value_error()?;
        Ok(Some(table))
    };
    Ok(Arc::new(ScriptedPermissions {
        answers: Mutex::new(VecDeque::from([answer])),
        modes: Mutex::new(Vec::new()),
    }))
}

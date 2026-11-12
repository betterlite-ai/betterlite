use std::sync::{Mutex, PoisonError};

use betterlite_context::non_default_params::Params;
use betterlite_core::api::responses_parameters::error::{
    ResponsesParametersError, UNSUPPORTED_PARAMS_STATUS,
};
use betterlite_core::api::responses_parameters::optional::optional_params_responses;
use betterlite_core::api::responses_parameters::provider::{DropPolicy, ResponsesParamProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::helper_error;
use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
pub(crate) struct OptionalInput {
    pub(super) steps: Vec<OptionalStep>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OptionalStep {
    model: String,
    supported: Option<Vec<String>>,
    allowed: Vec<String>,
    global_drop: bool,
    call_drop: Option<bool>,
    params: Params,
    mapped: Option<Params>,
}

#[derive(Debug, Clone, Serialize)]
struct MappingCall {
    model: String,
    drop_params: bool,
    params: Params,
}

struct Fixture {
    supported: Option<Vec<String>>,
    mapped: Option<Params>,
    calls: Mutex<Vec<MappingCall>>,
}

impl ResponsesParamProvider for Fixture {
    type Error = ResponsesParametersError;

    fn name(&self) -> &'static str {
        "openai"
    }

    fn supported_params(&self, _model: &str) -> Result<Option<Vec<String>>, Self::Error> {
        Ok(self.supported.clone())
    }

    fn map_params(
        &self,
        params: &Params,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, Self::Error> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(MappingCall {
            model: model.to_owned(),
            drop_params,
            params: params.clone(),
        });
        drop(calls);
        Ok(self.mapped.clone().unwrap_or_else(|| params.clone()))
    }
}

pub(super) fn step(fields: &OptionalStep) -> Result<Value, Box<HelperError>> {
    let mut params = fields.params.clone();
    let fixture = Fixture {
        supported: fields.supported.clone(),
        mapped: fields.mapped.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let result = optional_params_responses(
        &fixture,
        &fields.model,
        &mut params,
        &fields.allowed,
        DropPolicy {
            global: fields.global_drop,
            call: fields.call_drop,
        },
    );
    let calls = fixture
        .calls
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let (ok, status, value) = match result {
        Ok(mapped) => (true, None, json!(mapped)),
        Err(ResponsesParametersError::UnsupportedParams { .. }) => {
            (false, Some(UNSUPPORTED_PARAMS_STATUS), Value::Null)
        }
        Err(error) => return Err(helper_error(&error)),
    };
    Ok(json!({"ok": ok, "status": status, "value": value, "calls": calls, "params_after": params}))
}

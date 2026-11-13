use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use betterlite_openai::evals::transformation::{
    BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext,
};
use betterlite_openai::openai_base_config::Object;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::requests::{
    create_run_value, create_update_value, eval_id_value, list_evals_value, list_runs_value,
    run_id_value,
};
use super::{Call, LitellmParams, Probe, ReplyOperation, invalid_input};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

pub(super) type HeaderMap = BTreeMap<String, String>;

pub(super) fn run(call: &Call) -> HelperResult {
    match call {
        Call::EvalCreateUpdate(probes) => probe_values(&probes.probes, create_update_value),
        Call::EvalListPagination(probes) => probe_values(&probes.probes, list_evals_value),
        Call::EvalGetDeleteCancel(probes) => probe_values(&probes.probes, eval_id_value),
        Call::RunCreateDataSource(probes) => probe_values(&probes.probes, create_run_value),
        Call::RunListPagination(probes) => probe_values(&probes.probes, list_runs_value),
        Call::RunGetCancelDelete(probes) => probe_values(&probes.probes, run_id_value),
        Call::MalformedJsonError(probes) => probe_values(&probes.probes, reply_value),
        Call::EvalAuthRequired(probes) => probe_values(&probes.probes, auth_value),
        Call::EvalUrlLiteralAppend(probes) => probe_values(&probes.probes, url_value),
    }
}

fn probe_values(
    probes: &[Probe],
    value_of: fn(&Probe) -> Result<Value, Box<HelperError>>,
) -> HelperResult {
    let mut values = Vec::with_capacity(probes.len());
    for probe in probes {
        values.push(value_of(probe)?);
    }
    Ok(Value::Array(values))
}

pub(super) fn params_object(params: Option<&LitellmParams>) -> Result<Object, Box<HelperError>> {
    let Some(params) = params else {
        return Ok(Object::new());
    };
    match serde_json::to_value(params).map_err(|error| invalid_input(error.to_string()))? {
        Value::Object(fields) => Ok(fields),
        _ => Err(invalid_input(String::from(
            "litellm_params is not an object",
        ))),
    }
}

pub(super) fn context_of<'call>(
    params: &'call Object,
    headers: &'call HeaderMap,
) -> EvalsRequestContext<'call> {
    EvalsRequestContext {
        litellm_params: params,
        headers,
    }
}

fn reply_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let operation = probe
        .reply
        .ok_or_else(|| invalid_input(String::from("a reply probe needs reply")))?;
    let status = probe
        .status
        .ok_or_else(|| invalid_input(String::from("a reply probe needs status")))?;
    let text = probe
        .text
        .as_deref()
        .ok_or_else(|| invalid_input(String::from("a reply probe needs text")))?;
    let headers = probe.headers.clone().unwrap_or_default();
    let response = EvalsHttpResponse {
        status_code: status,
        headers: &headers,
        text,
    };
    match reply_result(operation, &response) {
        Ok(()) => Ok(json!({"value": null})),
        Err(_error) => Ok(json!({"error": {"status": null}})),
    }
}

fn reply_result(
    operation: ReplyOperation,
    response: &EvalsHttpResponse<'_>,
) -> Result<(), EvalsError> {
    let config = OpenAiEvalsConfig::default();
    match operation {
        ReplyOperation::CreateEval => config.transform_create_eval_response(response).map(|_| ()),
        ReplyOperation::ListEvals => config.transform_list_evals_response(response).map(|_| ()),
        ReplyOperation::GetEval => config.transform_get_eval_response(response).map(|_| ()),
        ReplyOperation::UpdateEval => config.transform_update_eval_response(response).map(|_| ()),
        ReplyOperation::DeleteEval => config.transform_delete_eval_response(response).map(|_| ()),
        ReplyOperation::CancelEval => config.transform_cancel_eval_response(response).map(|_| ()),
        ReplyOperation::CreateRun => config.transform_create_run_response(response).map(|_| ()),
        ReplyOperation::ListRuns => config.transform_list_runs_response(response).map(|_| ()),
        ReplyOperation::GetRun => config.transform_get_run_response(response).map(|_| ()),
        ReplyOperation::CancelRun => config.transform_cancel_run_response(response).map(|_| ()),
        ReplyOperation::DeleteRun => config.transform_delete_run_response(response).map(|_| ()),
    }
}

fn auth_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let env = probe.env.clone().unwrap_or_default();
    let config = ConfigLoader { lookup: env }.load("").or_config_error()?;
    let evals = OpenAiEvalsConfig {
        api_key: probe.litellm_api_key.clone().map(SecretString::from),
        openai_key: probe.litellm_openai_key.clone().map(SecretString::from),
        env_api_key: config.sections.resource_sdk_settings.openai_api_key.clone(),
    };
    let params = match &probe.litellm_params {
        Some(fields) => Some(params_object(Some(fields))?),
        None => None,
    };
    let headers = probe.headers.clone().unwrap_or_default();
    match evals.validate_environment(headers, params.as_ref()) {
        Ok(headers) => Ok(json!({"headers": headers})),
        Err(_error) => Ok(json!({"error": {"status": null}})),
    }
}

fn url_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let endpoint = probe
        .endpoint
        .as_deref()
        .ok_or_else(|| invalid_input(String::from("a url probe needs endpoint")))?;
    match OpenAiEvalsConfig::default().get_complete_url(
        probe.api_base.as_deref(),
        endpoint,
        probe.eval_id.as_deref(),
    ) {
        Ok(url) => Ok(json!({"url": url})),
        Err(_error) => Ok(json!({"error": {"status": null}})),
    }
}

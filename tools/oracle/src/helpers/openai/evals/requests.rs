use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use betterlite_openai::evals::transformation::BaseEvalsAPIConfig;
use betterlite_openai::evals::types::{CreateEvalRequest, CreateRunRequest, UpdateEvalRequest};
use betterlite_openai::openai_base_config::Object;
use serde_json::{Value, json};

use super::calls::{HeaderMap, context_of, outcome, params_object};
use super::{Operation, Probe, invalid_input};
use crate::helpers::HelperError;

pub(super) fn create_update_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let headers = probe.headers.clone().unwrap_or_default();
    if let Some(create) = &probe.create {
        return Ok(create_eval_value(create, &headers));
    }
    if let Some(update) = &probe.update {
        return Ok(update_eval_value(probe, update, &headers));
    }
    let Some(create_run) = &probe.create_run else {
        return Err(invalid_input(String::from(
            "a create-update probe needs create, update or create_run",
        )));
    };
    Ok(create_run_body_value(probe, create_run, &headers))
}

fn create_eval_value(create: &CreateEvalRequest, headers: &HeaderMap) -> Value {
    let params = Object::new();
    let context = context_of(&params, headers);
    match OpenAiEvalsConfig::default().transform_create_eval_request(create, &context) {
        Ok(body) => json!({"create": Value::Object(body)}),
        Err(_error) => json!({"error": {"status": null}}),
    }
}

fn update_eval_value(probe: &Probe, update: &UpdateEvalRequest, headers: &HeaderMap) -> Value {
    let (Some(eval_id), Some(api_base)) = (probe.eval_id.as_deref(), probe.api_base.as_deref())
    else {
        return json!({"error": {"status": null}});
    };
    let params = match params_object(probe.litellm_params.as_ref()) {
        Ok(params) => params,
        Err(_error) => return json!({"error": {"status": null}}),
    };
    let context = context_of(&params, headers);
    match OpenAiEvalsConfig::default()
        .transform_update_eval_request(eval_id, update, api_base, &context)
    {
        Ok((url, sent, body)) => {
            json!({"update": {"url": url, "headers": sent, "body": Value::Object(body)}})
        }
        Err(_error) => json!({"error": {"status": null}}),
    }
}

fn create_run_body_value(
    probe: &Probe,
    create_run: &CreateRunRequest,
    headers: &HeaderMap,
) -> Value {
    let Some(eval_id) = probe.eval_id.as_deref() else {
        return json!({"error": {"status": null}});
    };
    let params = match params_object(probe.litellm_params.as_ref()) {
        Ok(params) => params,
        Err(_error) => return json!({"error": {"status": null}}),
    };
    let context = context_of(&params, headers);
    match OpenAiEvalsConfig::default().transform_create_run_request(eval_id, create_run, &context) {
        Ok((url, body)) => json!({"create_run": {"url": url, "body": Value::Object(body)}}),
        Err(_error) => json!({"error": {"status": null}}),
    }
}

#[rustfmt::skip]
pub(super) fn list_evals_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let params = probe.list_params.as_ref().ok_or_else(|| invalid_input(String::from("a list probe needs list_params")))?;
    let litellm = params_object(probe.litellm_params.as_ref())?;
    let headers = probe.headers.clone().unwrap_or_default();
    let sent = OpenAiEvalsConfig::default().transform_list_evals_request(params, &context_of(&litellm, &headers));
    Ok(outcome(sent.map(|(url, query)| json!({"url": url, "query": Value::Object(query)}))))
}

struct Lifecycle<'probe> {
    operation: Operation,
    eval_id: &'probe str,
    api_base: &'probe str,
    params: Object,
    headers: HeaderMap,
}

fn lifecycle<'probe>(
    probe: &'probe Probe,
    owner: &str,
) -> Result<Lifecycle<'probe>, Box<HelperError>> {
    let operation = probe
        .operation
        .ok_or_else(|| invalid_input(format!("{owner} needs operation")))?;
    let eval_id = probe
        .eval_id
        .as_deref()
        .ok_or_else(|| invalid_input(format!("{owner} needs eval_id")))?;
    let api_base = probe
        .api_base
        .as_deref()
        .ok_or_else(|| invalid_input(format!("{owner} needs api_base")))?;
    Ok(Lifecycle {
        operation,
        eval_id,
        api_base,
        params: params_object(probe.litellm_params.as_ref())?,
        headers: probe.headers.clone().unwrap_or_default(),
    })
}

type Sent = (String, HeaderMap, Option<Object>);

fn sent_value(result: Result<Sent, EvalsError>) -> Value {
    match result {
        Ok((url, headers, None)) => json!({"url": url, "headers": headers}),
        Ok((url, headers, Some(body))) => {
            json!({"url": url, "headers": headers, "body": Value::Object(body)})
        }
        Err(_error) => json!({"error": {"status": null}}),
    }
}

pub(super) fn eval_id_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let call = lifecycle(probe, "an eval probe")?;
    Ok(sent_value(lifecycle_sent(&call, None)))
}

fn lifecycle_sent(call: &Lifecycle<'_>, run_id: Option<&str>) -> Result<Sent, EvalsError> {
    let context = context_of(&call.params, &call.headers);
    let config = OpenAiEvalsConfig::default();
    let (eval_id, api_base) = (call.eval_id, call.api_base);
    match (call.operation, run_id) {
        (Operation::Get, None) => config
            .transform_get_eval_request(eval_id, api_base, &context)
            .map(|(url, sent)| (url, sent, None)),
        (Operation::Delete, None) => config
            .transform_delete_eval_request(eval_id, api_base, &context)
            .map(|(url, sent)| (url, sent, None)),
        (Operation::Cancel, None) => config
            .transform_cancel_eval_request(eval_id, api_base, &context)
            .map(|(url, sent, body)| (url, sent, Some(body))),
        (Operation::Get, Some(run_id)) => config
            .transform_get_run_request(eval_id, run_id, api_base, &context)
            .map(|(url, sent)| (url, sent, None)),
        (Operation::Delete, Some(run_id)) => config
            .transform_delete_run_request(eval_id, run_id, api_base, &context)
            .map(|(url, sent, body)| (url, sent, Some(body))),
        (Operation::Cancel, Some(run_id)) => config
            .transform_cancel_run_request(eval_id, run_id, api_base, &context)
            .map(|(url, sent, body)| (url, sent, Some(body))),
    }
}

#[rustfmt::skip]
pub(super) fn create_run_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let create_run = probe.create_run.as_ref().ok_or_else(|| invalid_input(String::from("a run create probe needs create_run")))?;
    let litellm = params_object(probe.litellm_params.as_ref())?;
    let headers = probe.headers.clone().unwrap_or_default();
    let sent = OpenAiEvalsConfig::default().transform_create_run_request(probe.eval_id.as_deref().unwrap_or_default(), create_run, &context_of(&litellm, &headers));
    Ok(outcome(sent.map(|(url, body)| json!({"url": url, "body": Value::Object(body)}))))
}

#[rustfmt::skip]
pub(super) fn list_runs_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let params = probe.list_runs_params.as_ref().ok_or_else(|| invalid_input(String::from("a run list probe needs list_runs_params")))?;
    let litellm = params_object(probe.litellm_params.as_ref())?;
    let headers = probe.headers.clone().unwrap_or_default();
    let sent = OpenAiEvalsConfig::default().transform_list_runs_request(probe.eval_id.as_deref().unwrap_or_default(), params, &context_of(&litellm, &headers));
    Ok(outcome(sent.map(|(url, query)| json!({"url": url, "query": Value::Object(query)}))))
}

pub(super) fn run_id_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let call = lifecycle(probe, "a run probe")?;
    let run_id = probe
        .run_id
        .as_deref()
        .ok_or_else(|| invalid_input(String::from("a run probe needs run_id")))?;
    Ok(sent_value(lifecycle_sent(&call, Some(run_id))))
}

use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use betterlite_openai::evals::transformation::{BaseEvalsAPIConfig, EvalsRequestContext};
use betterlite_openai::evals::types::{CreateEvalRequest, CreateRunRequest, UpdateEvalRequest};
use betterlite_openai::openai_base_config::Object;
use serde_json::{Value, json};

use super::calls::{HeaderMap, context_of, params_object};
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

pub(super) fn list_evals_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let params = probe
        .list_params
        .as_ref()
        .ok_or_else(|| invalid_input(String::from("a list probe needs list_params")))?;
    let litellm = params_object(probe.litellm_params.as_ref())?;
    let headers = probe.headers.clone().unwrap_or_default();
    let context = context_of(&litellm, &headers);
    match OpenAiEvalsConfig::default().transform_list_evals_request(params, &context) {
        Ok((url, query)) => Ok(json!({"url": url, "query": Value::Object(query)})),
        Err(_error) => Ok(json!({"error": {"status": null}})),
    }
}

pub(super) fn eval_id_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let operation = probe
        .operation
        .ok_or_else(|| invalid_input(String::from("an eval probe needs operation")))?;
    let eval_id = probe
        .eval_id
        .as_deref()
        .ok_or_else(|| invalid_input(String::from("an eval probe needs eval_id")))?;
    let api_base = probe
        .api_base
        .as_deref()
        .ok_or_else(|| invalid_input(String::from("an eval probe needs api_base")))?;
    let params = params_object(probe.litellm_params.as_ref())?;
    let headers = probe.headers.clone().unwrap_or_default();
    let context = context_of(&params, &headers);
    let config = OpenAiEvalsConfig::default();
    Ok(match operation {
        Operation::Get => eval_get_value(&config, eval_id, api_base, &context),
        Operation::Delete => eval_delete_value(&config, eval_id, api_base, &context),
        Operation::Cancel => eval_cancel_value(&config, eval_id, api_base, &context),
    })
}

fn eval_get_value(
    config: &OpenAiEvalsConfig,
    eval_id: &str,
    api_base: &str,
    context: &EvalsRequestContext<'_>,
) -> Value {
    match config.transform_get_eval_request(eval_id, api_base, context) {
        Ok((url, sent)) => json!({"url": url, "headers": sent}),
        Err(_error) => json!({"error": {"status": null}}),
    }
}

fn eval_delete_value(
    config: &OpenAiEvalsConfig,
    eval_id: &str,
    api_base: &str,
    context: &EvalsRequestContext<'_>,
) -> Value {
    match config.transform_delete_eval_request(eval_id, api_base, context) {
        Ok((url, sent)) => json!({"url": url, "headers": sent}),
        Err(_error) => json!({"error": {"status": null}}),
    }
}

fn eval_cancel_value(
    config: &OpenAiEvalsConfig,
    eval_id: &str,
    api_base: &str,
    context: &EvalsRequestContext<'_>,
) -> Value {
    match config.transform_cancel_eval_request(eval_id, api_base, context) {
        Ok((url, sent, body)) => {
            json!({"url": url, "headers": sent, "body": Value::Object(body)})
        }
        Err(_error) => json!({"error": {"status": null}}),
    }
}

pub(super) fn create_run_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let create_run = probe
        .create_run
        .as_ref()
        .ok_or_else(|| invalid_input(String::from("a run create probe needs create_run")))?;
    let params = params_object(probe.litellm_params.as_ref())?;
    let headers = probe.headers.clone().unwrap_or_default();
    let context = context_of(&params, &headers);
    let Some(eval_id) = probe.eval_id.as_deref() else {
        return Ok(json!({"error": {"status": null}}));
    };
    match OpenAiEvalsConfig::default().transform_create_run_request(eval_id, create_run, &context) {
        Ok((url, body)) => Ok(json!({"url": url, "body": Value::Object(body)})),
        Err(_error) => Ok(json!({"error": {"status": null}})),
    }
}

pub(super) fn list_runs_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let params = probe
        .list_runs_params
        .as_ref()
        .ok_or_else(|| invalid_input(String::from("a run list probe needs list_runs_params")))?;
    let litellm = params_object(probe.litellm_params.as_ref())?;
    let headers = probe.headers.clone().unwrap_or_default();
    let context = context_of(&litellm, &headers);
    let Some(eval_id) = probe.eval_id.as_deref() else {
        return Ok(json!({"error": {"status": null}}));
    };
    match OpenAiEvalsConfig::default().transform_list_runs_request(eval_id, params, &context) {
        Ok((url, query)) => Ok(json!({"url": url, "query": Value::Object(query)})),
        Err(_error) => Ok(json!({"error": {"status": null}})),
    }
}

pub(super) fn run_id_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let operation = probe
        .operation
        .ok_or_else(|| invalid_input(String::from("a run probe needs operation")))?;
    let eval_id = probe
        .eval_id
        .as_deref()
        .ok_or_else(|| invalid_input(String::from("a run probe needs eval_id")))?;
    let run_id = probe
        .run_id
        .as_deref()
        .ok_or_else(|| invalid_input(String::from("a run probe needs run_id")))?;
    let api_base = probe
        .api_base
        .as_deref()
        .ok_or_else(|| invalid_input(String::from("a run probe needs api_base")))?;
    let params = params_object(probe.litellm_params.as_ref())?;
    let headers = probe.headers.clone().unwrap_or_default();
    let context = context_of(&params, &headers);
    let config = OpenAiEvalsConfig::default();
    Ok(match operation {
        Operation::Get => run_get_value(&config, eval_id, run_id, api_base, &context),
        Operation::Delete => run_delete_value(&config, eval_id, run_id, api_base, &context),
        Operation::Cancel => run_cancel_value(&config, eval_id, run_id, api_base, &context),
    })
}

fn run_get_value(
    config: &OpenAiEvalsConfig,
    eval_id: &str,
    run_id: &str,
    api_base: &str,
    context: &EvalsRequestContext<'_>,
) -> Value {
    match config.transform_get_run_request(eval_id, run_id, api_base, context) {
        Ok((url, sent)) => json!({"url": url, "headers": sent}),
        Err(_error) => json!({"error": {"status": null}}),
    }
}

fn run_delete_value(
    config: &OpenAiEvalsConfig,
    eval_id: &str,
    run_id: &str,
    api_base: &str,
    context: &EvalsRequestContext<'_>,
) -> Value {
    match config.transform_delete_run_request(eval_id, run_id, api_base, context) {
        Ok((url, sent, body)) => {
            json!({"url": url, "headers": sent, "body": Value::Object(body)})
        }
        Err(_error) => json!({"error": {"status": null}}),
    }
}

fn run_cancel_value(
    config: &OpenAiEvalsConfig,
    eval_id: &str,
    run_id: &str,
    api_base: &str,
    context: &EvalsRequestContext<'_>,
) -> Value {
    match config.transform_cancel_run_request(eval_id, run_id, api_base, context) {
        Ok((url, sent, body)) => {
            json!({"url": url, "headers": sent, "body": Value::Object(body)})
        }
        Err(_error) => json!({"error": {"status": null}}),
    }
}

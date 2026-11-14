use betterlite_openai::fireworks_ai_chat_params::FireworksChatParamsError;
use betterlite_openai::fireworks_ai_chat_params::extra_body::map_extra_body_params;
use betterlite_openai::fireworks_ai_chat_params::mapping::map_openai_params;
use betterlite_openai::fireworks_ai_chat_params::settings::FireworksChatSettings;
use betterlite_openai::fireworks_ai_chat_params::supported::supported_openai_params;
use betterlite_openai::openai_base_config::Object;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::fixture::Fixture;
use super::input::{Input, Step};

pub(crate) fn run(input: &Input) -> HelperResult {
    let mut fixture = Fixture::load(&input.catalog);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })?;
    runtime.block_on(execute(input, &mut fixture))
}

async fn execute(input: &Input, fixture: &mut Fixture) -> HelperResult {
    let mut outcomes = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        outcomes.push(step_value(step, fixture).await?);
    }
    Ok(Value::Array(outcomes))
}

async fn step_value(step: &Step, fixture: &mut Fixture) -> Result<Value, Box<HelperError>> {
    match step {
        Step::SupportedParams { model } => {
            let catalog = fixture.entries.clone();
            let params = supported_openai_params(fixture, &catalog, model).await;
            Ok(json!(params))
        }
        Step::MapParams {
            model,
            non_default_params,
            ordered_params,
            optional_params,
        } => {
            let pairs = params_in_order(non_default_params, ordered_params);
            let catalog = fixture.entries.clone();
            let mut optional = optional_params.clone();
            map_openai_params(fixture, &catalog, model, &pairs, &mut optional)
                .await
                .map_err(|error| helper_error(&error))?;
            Ok(Value::Object(optional))
        }
        Step::MapExtraBody {
            model,
            optional_params,
        } => {
            let mapped = map_extra_body_params(fixture, model, optional_params).await;
            Ok(Value::Object(mapped))
        }
        Step::Config { settings } => {
            let config = FireworksChatSettings::with_settings(settings.clone())
                .map_err(|error| helper_error(&error))?;
            Ok(Value::Object(config.get_config()))
        }
    }
}

fn params_in_order(non_default: &Object, ordered: &[(String, Value)]) -> Vec<(String, Value)> {
    let mut pairs: Vec<(String, Value)> = Vec::new();
    for (name, value) in non_default {
        insert_pair(&mut pairs, name, value);
    }
    for (name, value) in ordered {
        insert_pair(&mut pairs, name, value);
    }
    pairs
}

// a dict keeps the first position of a repeated key, which ordered_params may repeat
fn insert_pair(pairs: &mut Vec<(String, Value)>, name: &str, value: &Value) {
    if let Some((_, stored)) = pairs.iter_mut().find(|(key, _)| key == name) {
        *stored = value.clone();
    } else {
        pairs.push((name.to_owned(), value.clone()));
    }
}

fn helper_error(error: &FireworksChatParamsError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("FireworksChatParamsError"),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

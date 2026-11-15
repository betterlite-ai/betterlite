mod doubles;

use std::convert::Infallible;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::PassThroughEndpointHandler;
use betterlite_core::passthrough_guardrail::settings::{
    PassThroughGuardrailSettings, PassthroughGuardrailInputs, PassthroughInputType, RequestData,
    ResponseCall,
};
use betterlite_core::passthrough_guardrail::traits::{
    PassthroughGuardrail, PassthroughLogging, PassthroughPolicy,
};
use betterlite_guardrails::{guardrail_translation, passthrough_policy};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use doubles::{Policy, RecordingGuardrail, ScenarioLogging};

#[derive(Debug, Deserialize)]
struct LoggingSpec {
    config: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Scenario {
    Settings {
        guardrail_name: Option<String>,
        logging: Option<LoggingSpec>,
    },
    Extract {
        data: RequestData,
        field_expressions: Option<Vec<String>>,
    },
    Input {
        guardrail_name: Option<String>,
        logging: Option<LoggingSpec>,
        data: RequestData,
    },
    Output {
        guardrail_name: Option<String>,
        logging: Option<LoggingSpec>,
        response: GuardrailValue,
        request_data: Option<RequestData>,
        user_api_key: Option<RequestData>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        runtime.block_on(async {
            let mut answers = Vec::with_capacity(input.scenarios.len());
            for scenario in &input.scenarios {
                answers.push(answer(scenario).await?);
            }
            Ok(Value::Array(answers))
        })
    }
}

async fn answer(scenario: &Scenario) -> HelperResult {
    match scenario {
        Scenario::Settings {
            guardrail_name,
            logging,
        } => Ok(settings_answer(guardrail_name.as_deref(), logging.as_ref())),
        Scenario::Extract {
            data,
            field_expressions,
        } => extract_answer(data, field_expressions.as_deref()),
        Scenario::Input {
            guardrail_name,
            logging,
            data,
        } => input_answer(guardrail_name.as_deref(), logging.as_ref(), data.clone()).await,
        Scenario::Output {
            guardrail_name,
            logging,
            response,
            request_data,
            user_api_key,
        } => {
            output_answer(
                guardrail_name.as_deref(),
                logging.as_ref(),
                response,
                request_data.as_ref(),
                user_api_key.as_ref(),
            )
            .await
        }
    }
}

fn settings_answer(guardrail_name: Option<&str>, logging: Option<&LoggingSpec>) -> Value {
    let logging_obj = logging.map(|spec| ScenarioLogging {
        config: spec.config.clone(),
    });
    match PassThroughEndpointHandler::get_guardrail_settings(
        logging_obj.as_ref(),
        guardrail_name,
        &Policy,
    ) {
        None => Value::Null,
        Some(settings) => json!({
            "request_fields": settings.request_fields,
            "response_fields": settings.response_fields,
        }),
    }
}

fn extract_answer(data: &RequestData, field_expressions: Option<&[String]>) -> HelperResult {
    let text =
        PassThroughEndpointHandler::extract_text_for_guardrail(data, field_expressions, &Policy)
            .map_err(failure)?;
    Ok(normalised_text(&text))
}

async fn input_answer(
    guardrail_name: Option<&str>,
    logging: Option<&LoggingSpec>,
    mut data: RequestData,
) -> HelperResult {
    let guardrail = RecordingGuardrail {
        name: guardrail_name.map(str::to_owned),
        calls: Mutex::new(Vec::new()),
    };
    let mut logging_obj = logging.map(|spec| ScenarioLogging {
        config: spec.config.clone(),
    });
    PassThroughEndpointHandler::process_input_messages(
        &mut data,
        &guardrail,
        logging_obj.as_mut(),
        &Policy,
    )
    .await
    .map_err(failure)?;
    Ok(json!({ "returned": data, "calls": guardrail.recorded() }))
}

async fn output_answer(
    guardrail_name: Option<&str>,
    logging: Option<&LoggingSpec>,
    response: &GuardrailValue,
    request_data: Option<&RequestData>,
    user_api_key: Option<&RequestData>,
) -> HelperResult {
    let guardrail = RecordingGuardrail {
        name: guardrail_name.map(str::to_owned),
        calls: Mutex::new(Vec::new()),
    };
    let mut logging_obj = logging.map(|spec| ScenarioLogging {
        config: spec.config.clone(),
    });
    let mut owned_request_data = request_data.cloned();
    let returned = PassThroughEndpointHandler::process_output_response(
        response.clone(),
        &guardrail,
        ResponseCall {
            logging_obj: logging_obj.as_mut(),
            user_api_key_dict: user_api_key,
            request_data: owned_request_data.as_mut(),
        },
        &Policy,
    )
    .await
    .map_err(failure)?;
    let mut reply = json!({ "returned": returned, "calls": guardrail.recorded() });
    if let (Some(fields), Some(updated)) = (reply.as_object_mut(), &owned_request_data) {
        fields.insert("request_data".to_owned(), json!(updated));
    }
    Ok(reply)
}

// the same split as the Python helper: a payload that parses as JSON is compared as JSON
fn normalised_text(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => value,
        Err(_error) => Value::String(text.to_owned()),
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "PassthroughGuardrailError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

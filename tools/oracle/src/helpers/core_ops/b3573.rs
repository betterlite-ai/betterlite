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
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

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
        let runtime = tokio::runtime::Builder::new_current_thread()
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
                request_data,
                user_api_key,
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
    request_data: &Option<RequestData>,
    user_api_key: &Option<RequestData>,
) -> HelperResult {
    let guardrail = RecordingGuardrail {
        name: guardrail_name.map(str::to_owned),
        calls: Mutex::new(Vec::new()),
    };
    let mut logging_obj = logging.map(|spec| ScenarioLogging {
        config: spec.config.clone(),
    });
    let mut owned_request_data = request_data.clone();
    let returned = PassThroughEndpointHandler::process_output_response(
        response.clone(),
        &guardrail,
        ResponseCall {
            logging_obj: logging_obj.as_mut(),
            user_api_key_dict: user_api_key.as_ref(),
            request_data: owned_request_data.as_mut(),
        },
        &Policy,
    )
    .await
    .map_err(failure)?;
    let mut reply = json!({ "returned": returned, "calls": guardrail.recorded() });
    if let Some(request_data) = &owned_request_data {
        reply["request_data"] = json!(request_data);
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

struct ScenarioLogging {
    config: Option<Json>,
}

impl PassthroughLogging for ScenarioLogging {
    fn passthrough_guardrails_config(&self) -> Option<&Json> {
        self.config.as_ref()
    }
}

// the no-op reply keeps the check-only contract visible; the calls are what the cases compare
struct RecordingGuardrail {
    name: Option<String>,
    calls: Mutex<Vec<Value>>,
}

impl RecordingGuardrail {
    fn recorded(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PassthroughGuardrail for RecordingGuardrail {
    type Failure = Infallible;
    type LogTarget = ScenarioLogging;

    fn guardrail_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn apply_guardrail(
        &self,
        inputs: PassthroughGuardrailInputs,
        request_data: &mut RequestData,
        input_type: PassthroughInputType,
        _logging_obj: Option<&mut ScenarioLogging>,
    ) -> impl Future<Output = Result<PassthroughGuardrailInputs, Infallible>> + Send {
        let kind = match input_type {
            PassthroughInputType::Request => "request",
            PassthroughInputType::Response => "response",
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "texts": inputs.texts,
                "model": inputs.model,
                "input_type": kind,
                "request_data": request_data,
            }));
        ready(Ok(inputs))
    }
}

// B3601's passthrough policy lives in betterlite-guardrails, which depends on betterlite-core
struct Policy;

impl PassthroughPolicy for Policy {
    fn get_settings(
        &self,
        config: &Json,
        guardrail_name: &str,
    ) -> Option<PassThroughGuardrailSettings> {
        betterlite_guardrails::passthrough_policy::get_settings(Some(config), guardrail_name).map(
            |settings| PassThroughGuardrailSettings {
                request_fields: settings.request_fields,
                response_fields: settings.response_fields,
            },
        )
    }

    fn extract_fields(&self, data: &Json, expressions: &[String]) -> String {
        betterlite_guardrails::passthrough_policy::extract_fields(data, expressions)
    }

    fn user_key_metadata(&self, user_api_key_dict: Option<&RequestData>) -> RequestData {
        betterlite_guardrails::guardrail_translation::transform_user_key_metadata(user_api_key_dict)
    }
}

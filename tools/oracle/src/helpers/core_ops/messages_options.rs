use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_core::api::messages_options::error::MessagesOptionsError;
use betterlite_core::api::messages_options::mock::mock_message;
use betterlite_core::api::messages_options::refusal::{
    ChatTurn, OutputItem, chat_refusal_text, refusal_stop_details, responses_output_refusal_text,
    safeguard_refusal_error, safeguard_refusal_stop_details,
};
use betterlite_core::api::messages_options::{
    ModelGate, requested_optional_params, system_message,
};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
struct Fixture {
    model_cost: Value,
    packaged: Value,
}

#[derive(Debug, Deserialize)]
struct OptionalParams {
    params: IndexMap<String, Json>,
    model: Option<String>,
    custom_llm_provider: Option<String>,
    drop_params: bool,
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Item {
    Typed { value: Box<SdkOutput> },
    Raw { value: Json },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    #[serde(rename = "get_requested_anthropic_messages_optional_param")]
    OptionalParam(OptionalParams),
    GetSafeguardRefusalStopDetails {
        response: Json,
    },
    RefusalStopDetails {
        explanation: Option<String>,
    },
    SafeguardRefusalError {
        model: String,
        stop_details: IndexMap<String, Json>,
    },
    OpenaiChatRefusalText {
        refusal: Json,
        provider_specific_fields: Json,
    },
    ResponsesOutputRefusalText {
        output: Vec<Item>,
    },
    AnthropicSystemToOpenaiMessage {
        system: Json,
    },
    MockResponse {
        model: String,
        mock_response: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    fixture: Fixture,
    steps: Vec<Step>,
}

// What the oracle compares of an SDK exception; the gate message quotes its parameter with Python's repr, so it is left out.
#[derive(Serialize)]
struct Failure<'failure> {
    exception: &'static str,
    status: u16,
    #[serde(rename = "type")]
    error_type: WireField,
    param: WireField,
    code: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'failure str>,
}

fn exception_of(class: &ErrorClass) -> Result<(&'static str, Option<&str>), Box<HelperError>> {
    Ok(match class {
        ErrorClass::InternalServer(context) => {
            ("InternalServerError", Some(context.message.as_str()))
        }
        ErrorClass::ContextWindowExceeded(details) => {
            ("ContextWindowExceededError", Some(details.message.as_str()))
        }
        ErrorClass::ContentPolicyViolation(details) => (
            "ContentPolicyViolationError",
            Some(details.message.as_str()),
        ),
        ErrorClass::RateLimit(details) => {
            ("RateLimitError", Some(details.context.message.as_str()))
        }
        ErrorClass::UnsupportedParams(_) => ("UnsupportedParamsError", None),
        ErrorClass::Authentication(_)
        | ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::BadRequest(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::Timeout(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::ServiceUnavailable(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::Api(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => {
            return Err(format!("no projection for {class:?}")).or_value_error();
        }
    })
}

fn failure(class: &ErrorClass) -> Result<Value, Box<HelperError>> {
    let (exception, message) = exception_of(class)?;
    let metadata = class.metadata();
    let projected = serde_json::to_value(Failure {
        exception,
        status: metadata.status_code,
        error_type: metadata.error_type,
        param: metadata.param,
        code: metadata.openai_code,
        model: metadata.model,
        provider: metadata.provider,
        message,
    })
    .or_value_error()?;
    Ok(json!({"error": projected}))
}

fn raised(error: &MessagesOptionsError) -> Result<Value, Box<HelperError>> {
    let class = error
        .classify()
        .ok_or_else(|| error.to_string())
        .or_value_error()?;
    failure(&class)
}

impl OptionalParams {
    fn answer(&self, catalogs: &CatalogSources<'_>) -> Result<Value, Box<HelperError>> {
        let gate = self.model.as_deref().map(|model| ModelGate {
            model,
            drop: DropParams {
                global: self.global_drop_params,
                call: self.drop_params,
            },
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            catalogs,
        });
        match requested_optional_params(&self.params, gate.as_ref()) {
            Ok(kept) => Ok(json!({"params": kept.iter().collect::<Vec<_>>()})),
            Err(error) => raised(&error),
        }
    }
}

impl Step {
    fn answer(&self, catalogs: &CatalogSources<'_>) -> Result<Value, Box<HelperError>> {
        match self {
            Self::OptionalParam(call) => call.answer(catalogs),
            Self::GetSafeguardRefusalStopDetails { response } => {
                serde_json::to_value(safeguard_refusal_stop_details(response)).or_value_error()
            }
            Self::RefusalStopDetails { explanation } => {
                serde_json::to_value(refusal_stop_details(explanation.clone())).or_value_error()
            }
            Self::SafeguardRefusalError {
                model,
                stop_details,
            } => failure(&safeguard_refusal_error(model, stop_details)),
            Self::OpenaiChatRefusalText {
                refusal,
                provider_specific_fields,
            } => Ok(json!(chat_refusal_text(&ChatTurn {
                refusal,
                provider_specific_fields,
            }))),
            Self::ResponsesOutputRefusalText { output } => {
                let items: Vec<OutputItem<'_>> = output
                    .iter()
                    .map(|item| match item {
                        Item::Typed { value } => OutputItem::Typed(value),
                        Item::Raw { value } => OutputItem::Raw(value),
                    })
                    .collect();
                Ok(json!(responses_output_refusal_text(&items)))
            }
            Self::AnthropicSystemToOpenaiMessage { system } => {
                serde_json::to_value(system_message(system)).or_value_error()
            }
            Self::MockResponse {
                model,
                mock_response,
            } => match mock_message(model, mock_response.as_deref()) {
                Ok(message) => serde_json::to_value(message).or_value_error(),
                Err(error) => raised(&error),
            },
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let patterns = CandidatePatterns::new().or_config_error()?;
        let runtime: RawCatalog =
            serde_json::from_value(input.fixture.model_cost.clone()).or_config_error()?;
        let packaged: RawCatalog =
            serde_json::from_value(input.fixture.packaged.clone()).or_config_error()?;
        let catalogs = CatalogSources {
            patterns: &patterns,
            runtime: &runtime,
            packaged: &packaged,
        };
        let answers = input
            .steps
            .iter()
            .map(|step| step.answer(&catalogs))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(answers))
    }
}

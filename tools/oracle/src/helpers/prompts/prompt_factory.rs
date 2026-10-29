use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::builtin::{Field, Message};
use betterlite_prompts::error::FactoryError;
use betterlite_prompts::factory::custom::{
    CustomPromptTemplate, custom_prompt, default_response_schema_prompt, function_call_prompt,
    response_schema_prompt,
};
use betterlite_prompts::factory::{Output, prompt_factory};
use chrono::{TimeZone as _, Utc};
use serde::Deserialize;
use serde_json::{Value, to_value};

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    CustomPrompt {
        template: CustomPromptTemplate,
        messages: Vec<Message>,
    },
    FunctionCallPrompt {
        messages: Vec<Message>,
        functions: Vec<Field>,
    },
    ResponseSchemaPrompt {
        response_schema: Field,
        #[serde(default)]
        template: Option<CustomPromptTemplate>,
    },
    DefaultResponseSchemaPrompt {
        response_schema: Field,
    },
    Factory {
        model: String,
        messages: Vec<Message>,
        #[serde(default)]
        custom_llm_provider: Option<String>,
    },
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::CustomPrompt { template, messages } => custom_prompt(template, messages)
                .map(Value::from)
                .map_err(|error| failure(&error)),
            Input::FunctionCallPrompt {
                messages,
                functions,
            } => function_call_prompt(messages, functions)
                .map(|output| to_value(output).unwrap_or(Value::Null))
                .map_err(|error| failure(&error)),
            Input::ResponseSchemaPrompt {
                response_schema,
                template,
            } => response_schema_prompt(response_schema, template.as_ref())
                .map(Value::from)
                .map_err(|error| failure(&error)),
            Input::DefaultResponseSchemaPrompt { response_schema } => {
                default_response_schema_prompt(response_schema)
                    .map(Value::from)
                    .map_err(|error| failure(&error))
            }
            Input::Factory {
                model,
                messages,
                custom_llm_provider,
            } => {
                let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
                prompt_factory(model, messages, custom_llm_provider.as_deref(), now)
                    .map(output_value)
                    .map_err(|error| failure(&error))
            }
        }
    }
}

fn output_value(output: Output) -> Value {
    match output {
        Output::Text(text) => Value::from(text),
        Output::Ollama(prompt) => to_value(prompt).unwrap_or(Value::Null),
        Output::None => Value::Null,
    }
}

fn failure(error: &FactoryError) -> Box<HelperError> {
    let class = match error {
        FactoryError::Prompt(_) | FactoryError::Render(_) | FactoryError::NoChatTemplate => {
            "Exception"
        }
        FactoryError::Repr(_) => "SerializationError",
        FactoryError::Limit | FactoryError::InputLimit(_) => "LimitError",
        FactoryError::Append { .. } => "AttributeError",
        FactoryError::Unsupported { .. } => "NotBuilt",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;

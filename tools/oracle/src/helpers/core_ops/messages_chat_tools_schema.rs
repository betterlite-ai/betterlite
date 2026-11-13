use betterlite_core::api::messages_chat_tools_schema::output_format::{
    OutputFormatError, output_format_to_openai,
};
use betterlite_core::api::messages_chat_tools_schema::request::{
    translate_output_format, translate_tool_choice, translate_tools,
};
use betterlite_core::api::messages_chat_tools_schema::tool_choice::{
    ToolChoiceError, tool_choice_to_openai,
};
use betterlite_core::api::messages_chat_tools_schema::tools::{
    ToolNameMapping, ToolsError, tools_to_openai,
};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::to_value;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ToolChoice {
        tool_choice: Json,
    },
    Tools {
        tools: Vec<Json>,
        model: Option<String>,
    },
    OutputFormat {
        output_format: Json,
    },
    Request {
        request: Object,
    },
}

#[derive(Serialize)]
struct ToolsAnswer {
    tools: Vec<Json>,
    tool_name_mapping: ToolNameMapping,
}

#[derive(Serialize)]
struct FormatAnswer {
    response_format: Option<Json>,
    output_format: Json,
}

#[derive(Serialize)]
struct RequestAnswer {
    kwargs: Object,
    tool_name_mapping: ToolNameMapping,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = match input {
            Input::ToolChoice { tool_choice } => to_value(
                tool_choice_to_openai(tool_choice)
                    .map_err(|error| failure(choice_class(&error), None, error.to_string()))?,
            ),
            Input::Tools { tools, model } => to_value(tools_answer(tools, model.as_deref())?),
            Input::OutputFormat { output_format } => to_value(format_answer(output_format)?),
            Input::Request { request } => to_value(request_answer(request)?),
        };
        answer.map_err(|source| failure("SerializationError", None, source.to_string()))
    }
}

fn tools_answer(tools: &[Json], model: Option<&str>) -> Result<ToolsAnswer, Box<HelperError>> {
    let translation = tools_to_openai(tools, model).map_err(|error| tools_failure(&error))?;
    Ok(ToolsAnswer {
        tools: translation.tools,
        tool_name_mapping: translation.name_mapping,
    })
}

fn request_answer(request: &Object) -> Result<RequestAnswer, Box<HelperError>> {
    let Some(model) = request.get("model") else {
        return Err(failure("KeyError", None, "model".to_owned()));
    };
    let mut kwargs = Object::new();
    kwargs.insert("model".to_owned(), model.clone());
    let messages = request.get("messages").cloned();
    kwargs.insert(
        "messages".to_owned(),
        messages.unwrap_or_else(|| Json::List(Vec::new())),
    );
    translate_tool_choice(request, &mut kwargs)
        .map_err(|error| failure(choice_class(&error), None, error.to_string()))?;
    let tool_name_mapping =
        translate_tools(request, &mut kwargs).map_err(|error| tools_failure(&error))?;
    translate_output_format(request, &mut kwargs).map_err(|error| format_failure(&error))?;
    Ok(RequestAnswer {
        kwargs,
        tool_name_mapping,
    })
}

fn format_answer(output_format: &Json) -> Result<FormatAnswer, Box<HelperError>> {
    let response_format =
        output_format_to_openai(output_format).map_err(|error| format_failure(&error))?;
    Ok(FormatAnswer {
        response_format,
        output_format: output_format.clone(),
    })
}

const fn choice_class(error: &ToolChoiceError) -> &'static str {
    match error {
        ToolChoiceError::MissingType => "KeyError",
        ToolChoiceError::Unsupported => "ValueError",
        ToolChoiceError::Shape | ToolChoiceError::Name => "TypeError",
    }
}

fn tools_failure(error: &ToolsError) -> Box<HelperError> {
    if let ToolsError::EagerInput(source) = error {
        return failure(
            "UnsupportedParamsError",
            source.status_code(),
            format!("litellm.UnsupportedParamsError: {source}"),
        );
    }
    let class = match error {
        ToolsError::NotAnObject { .. } | ToolsError::TypeNotText { .. } => "AttributeError",
        ToolsError::NotAList
        | ToolsError::NameNotText { .. }
        | ToolsError::SchemaNotObject { .. }
        | ToolsError::ModelNotText
        | ToolsError::EagerInput(_) => "TypeError",
    };
    failure(class, None, error.to_string())
}

fn format_failure(error: &OutputFormatError) -> Box<HelperError> {
    let class = match error {
        OutputFormatError::Properties | OutputFormatError::Definitions { .. } => "AttributeError",
        OutputFormatError::SubschemaList { .. } => "TypeError",
    };
    failure(class, None, error.to_string())
}

fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::sdk::literals::WebSearchStatusChoice;
use betterlite_openai::api::responses_objects::sdk::output::{FunctionCall, FunctionCallTag};
use betterlite_openai::api::responses_objects::sdk::search::{
    ActionFind, ActionFindTag, ActionOpenPage, ActionOpenPageTag, ActionSearch, ActionSearchSource,
    ActionSearchSourceTag, ActionSearchTag, WebSearch, WebSearchTag,
};
use betterlite_openai::api::responses_objects::sdk::unions::{SdkOutput, WebSearchActionValue};
use betterlite_openai::api::responses_shapes::builders::WebSearchStatus;
use betterlite_openai::api::responses_shapes::{
    GenericResponseOutputItem, OutputCodeInterpreterCall, OutputImageGenerationCall,
};

use crate::api::responses_tool_namespaces::hosted_search::{HostedSearchCall, SearchAction};
use crate::api::responses_tool_namespaces::tool_outputs::{FunctionCallItem, ToolOutputItem};

use super::error::ChatOutputError;
use super::records::ChatChoice;

pub enum OutputItem {
    Reasoning(GenericResponseOutputItem),
    Message(GenericResponseOutputItem),
    Image(OutputImageGenerationCall),
    Tool(ToolOutputItem),
    CodeInterpreter(OutputCodeInterpreterCall),
}

#[must_use]
pub fn into_response_item(item: OutputItem) -> ResponseOutputItem {
    match item {
        OutputItem::Reasoning(output) | OutputItem::Message(output) => {
            ResponseOutputItem::Message(output)
        }
        OutputItem::Image(image) => ResponseOutputItem::ImageGeneration(image),
        OutputItem::CodeInterpreter(call) => ResponseOutputItem::CodeInterpreter(call),
        OutputItem::Tool(ToolOutputItem::Custom(custom)) => ResponseOutputItem::CustomTool(custom),
        OutputItem::Tool(ToolOutputItem::WebSearch(call)) => {
            ResponseOutputItem::Sdk(Box::new(web_search_item(call)))
        }
        OutputItem::Tool(ToolOutputItem::Function(call)) => {
            ResponseOutputItem::Sdk(Box::new(function_item(call)))
        }
    }
}

pub fn interpreter_results(
    choices: &[ChatChoice],
) -> Result<BTreeMap<String, OutputCodeInterpreterCall>, ChatOutputError> {
    let mut results = BTreeMap::new();
    for choice in choices {
        let Some(fields) = &choice.message.provider_specific_fields else {
            continue;
        };
        let Some(Json::List(items)) = fields.get("code_interpreter_results") else {
            continue;
        };
        for item in items {
            let result = interpreter_item(item)?;
            results.insert(result.id.clone(), result);
        }
    }
    Ok(results)
}

#[must_use]
pub fn replace_function_calls(
    items: Vec<OutputItem>,
    results: &BTreeMap<String, OutputCodeInterpreterCall>,
) -> Vec<OutputItem> {
    items
        .into_iter()
        .map(|item| match item {
            OutputItem::Tool(ToolOutputItem::Function(call)) => {
                results.get(&call.call_id).map_or_else(
                    || OutputItem::Tool(ToolOutputItem::Function(call)),
                    |result| OutputItem::CodeInterpreter(result.clone()),
                )
            }
            other @ (OutputItem::Reasoning(_)
            | OutputItem::Message(_)
            | OutputItem::Image(_)
            | OutputItem::Tool(ToolOutputItem::Custom(_) | ToolOutputItem::WebSearch(_))
            | OutputItem::CodeInterpreter(_)) => other,
        })
        .collect()
}

fn interpreter_item(item: &Json) -> Result<OutputCodeInterpreterCall, ChatOutputError> {
    if !matches!(item, Json::Object(_)) {
        return Err(ChatOutputError::WrongType {
            field: "code_interpreter_results",
            expected: "an object",
        });
    }
    let text = serde_json::to_string(item).map_err(|source| ChatOutputError::Json {
        field: "code_interpreter_results",
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| ChatOutputError::Json {
        field: "code_interpreter_results",
        source,
    })
}

fn function_item(call: FunctionCallItem) -> SdkOutput {
    let mut extra = BTreeMap::new();
    if let Some(fields) = call.provider_specific_fields {
        extra.insert("provider_specific_fields".to_owned(), Json::Object(fields));
    }
    SdkOutput::FunctionCall(Box::new(FunctionCall {
        r#type: FunctionCallTag::Value,
        arguments: call.arguments,
        call_id: call.call_id,
        name: call.name,
        id: Some(call.id),
        caller: None,
        namespace: call.namespace,
        status: Some(call.status),
        extra,
    }))
}

fn web_search_item(call: HostedSearchCall) -> SdkOutput {
    let action = search_action(call.action);
    let status = search_status(call.status);
    SdkOutput::WebSearch(Box::new(WebSearch {
        r#type: WebSearchTag::Value,
        id: call.id,
        action,
        status,
        extra: call.extra,
    }))
}

fn search_action(action: SearchAction) -> WebSearchActionValue {
    match action {
        SearchAction::Search {
            queries,
            query,
            sources,
            extra,
        } => WebSearchActionValue::ActionSearch(Box::new(ActionSearch {
            r#type: ActionSearchTag::Value,
            queries,
            query,
            sources: sources.map(|items| {
                items
                    .into_iter()
                    .map(|source| ActionSearchSource {
                        r#type: ActionSearchSourceTag::Value,
                        url: source.url,
                        extra: source.extra,
                    })
                    .collect()
            }),
            extra,
        })),
        SearchAction::OpenPage { url, extra } => {
            WebSearchActionValue::ActionOpenPage(Box::new(ActionOpenPage {
                r#type: ActionOpenPageTag::Value,
                url,
                extra,
            }))
        }
        SearchAction::Find {
            pattern,
            url,
            extra,
        } => WebSearchActionValue::ActionFind(Box::new(ActionFind {
            r#type: ActionFindTag::Value,
            pattern,
            url,
            extra,
        })),
    }
}

const fn search_status(status: WebSearchStatus) -> WebSearchStatusChoice {
    match status {
        WebSearchStatus::InProgress => WebSearchStatusChoice::InProgress,
        WebSearchStatus::Searching => WebSearchStatusChoice::Searching,
        WebSearchStatus::Completed => WebSearchStatusChoice::Completed,
        WebSearchStatus::Failed => WebSearchStatusChoice::Failed,
    }
}

use betterlite_core::api::response_chat_history::error::HistoryError;
use betterlite_core::api::response_chat_history::history::{HistoryMessage, convert_history};
use betterlite_core::api::response_chat_history::metadata::shared_output_metadata;
use betterlite_core::api::response_chat_history::reasoning::{
    ReasoningIds, ReasoningOutputItem, chat_reasoning_items, reasoning_input_items,
    reasoning_records,
};
use betterlite_core::api::response_chat_history::tool_fields::output_tool_fields;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    LeadingSystemInstructions {
        histories: Vec<Vec<HistoryMessage>>,
    },
    AssistantCallHistory {
        histories: Vec<Vec<HistoryMessage>>,
    },
    CorrelatedToolOutput {
        histories: Vec<Vec<HistoryMessage>>,
    },
    ReasoningHistory {
        messages: Vec<HistoryMessage>,
        output_item_groups: Vec<Vec<ReasoningOutputItem>>,
    },
    OutputToolFields {
        items: Vec<ToolFieldCase>,
    },
    SharedOutputMetadata {
        responses: Vec<MetadataResponse>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolFieldCase {
    item: Map<String, Value>,
    index: usize,
    #[serde(default)]
    provider_specific_fields: Option<ProviderFields>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProviderFields {
    Dict { value: Map<String, Value> },
    Attributes { region: String },
    Unsupported { value: Value },
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataResponse {
    #[serde(default)]
    fields: Option<Map<String, Value>>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    payload: Value,
}

pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::LeadingSystemInstructions { histories }
            | Input::AssistantCallHistory { histories }
            | Input::CorrelatedToolOutput { histories } => history_output(histories),
            Input::ReasoningHistory {
                messages,
                output_item_groups,
            } => Ok(reasoning_output(messages, output_item_groups)?),
            Input::OutputToolFields { items } => {
                let mut converted = Vec::new();
                for case in items {
                    converted.push(tool_field_case(case)?);
                }
                Ok(Value::Array(converted))
            }
            Input::SharedOutputMetadata { responses } => {
                Ok(Value::Array(metadata_output(responses)))
            }
        }
    }
}

fn history_output(
    histories: &[Vec<HistoryMessage>],
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut converted = Vec::new();
    for messages in histories {
        let history = convert_history(messages).map_err(|error| failure(&error))?;
        converted.push(json!({
            "input": history.input,
            "instructions": history.instructions,
        }));
    }
    Ok(Value::Array(converted))
}

fn reasoning_output(
    messages: &[HistoryMessage],
    output_item_groups: &[Vec<ReasoningOutputItem>],
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut ids = ReasoningIds::default();
    let mut projected = Vec::new();
    for message in messages {
        projected.push(reasoning_input_items(message, &mut ids).map_err(|error| failure(&error))?);
    }
    let mut groups = Vec::new();
    for output_items in output_item_groups {
        let records = reasoning_records(output_items).map_err(|error| failure(&error))?;
        let chat_items = chat_reasoning_items(&records);
        groups.push(json!({"built": records, "chat_items": chat_items}));
    }
    Ok(json!({"reasoning_input": projected, "groups": groups}))
}

fn tool_field_case(case: &ToolFieldCase) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut item = case.item.clone();
    let provider = match &case.provider_specific_fields {
        Some(ProviderFields::Dict { value }) => Some(Value::Object(value.clone())),
        Some(ProviderFields::Attributes { region }) => Some(json!({"region": region})),
        Some(ProviderFields::Unsupported { value }) => Some(value.clone()),
        None => None,
    };
    if let Some(value) = provider {
        item.insert("provider_specific_fields".to_owned(), value);
    }
    let converted = output_tool_fields(&item, case.index).map_err(|error| failure(&error))?;
    Ok(json!(converted))
}

fn metadata_output(responses: &[MetadataResponse]) -> Vec<Value> {
    let mut projections = Vec::new();
    for response in responses {
        let projection = shared_output_metadata(
            response.fields.as_ref(),
            response.id.as_deref(),
            &response.payload,
        );
        projections.push(json!({
            "metadata": projection.metadata,
            "upstream_response_id": projection.upstream_response_id,
            "incomplete_reason": projection.incomplete_reason,
            "incomplete_finish": projection.incomplete_finish,
        }));
    }
    projections
}

fn failure(error: &HistoryError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

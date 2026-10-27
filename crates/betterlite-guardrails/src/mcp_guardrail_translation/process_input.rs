use super::error::{ContentBlocked, NestedPayload, TranslationError};
use super::guardrail::{ApplyGuardrail, FunctionDefinition, GuardrailInputs, ToolDefinition};
use super::input_writeback::write_back;
use super::json_value::McpValue;
use super::nesting_depth::{Leaf, PathPart, scan_json};
use super::tool_request::McpToolRequest;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::generic_guardrail_contracts::headers::LoggingObjectContext;
use std::collections::BTreeMap;
mod tests;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputScan {
    pub inputs: GuardrailInputs,
    pub description: Option<String>,
    pub schema: BTreeMap<String, McpValue>,
    pub schema_leaves: Vec<Leaf>,
    pub argument_leaves: Vec<Leaf>,
}
pub async fn process_input_messages<Guardrail>(
    request: &mut McpToolRequest,
    guardrail: &Guardrail,
    logging_obj: Option<&mut LoggingObjectContext>,
) -> Result<(), TranslationError<Guardrail::Error>>
where
    Guardrail: ApplyGuardrail + Sync,
{
    let Some(scan) = InputScan::of(request)? else {
        tracing::debug!("MCP guardrail: mcp_tool_name missing");
        return Ok(());
    };
    let returned = guardrail
        .apply_guardrail(
            scan.inputs.clone(),
            Some(request),
            GuardrailInputType::Request,
            logging_obj,
        )
        .await
        .map_err(TranslationError::Guardrail)?;
    write_back(&scan, request, returned)?;
    Ok(())
}
impl InputScan {
    pub fn of(request: &McpToolRequest) -> Result<Option<Self>, ContentBlocked> {
        let Some(name) = request.tool_name() else {
            return Ok(None);
        };
        let description = request.tool_description().map(str::to_owned);
        let schema = match &request.mcp_input_schema {
            Some(McpValue::Object(entries)) => entries.clone(),
            Some(_) | None => BTreeMap::new(),
        };
        let schema_leaves: Vec<Leaf> = scan_json(
            &McpValue::Object(schema.clone()),
            NestedPayload::InputSchema,
        )?
        .leaves
        .into_iter()
        .filter(|leaf| leaf.path.0.last() == Some(&PathPart::Key("description".to_owned())))
        .collect();
        let argument_leaves = argument_leaves(request.tool_arguments())?;
        let texts = description
            .iter()
            .cloned()
            .chain(schema_leaves.iter().map(|leaf| leaf.text.clone()))
            .chain(argument_leaves.iter().map(|leaf| leaf.text.clone()))
            .collect();
        let tool = tool_definition(name, description.as_deref(), &schema);
        Ok(Some(Self {
            inputs: GuardrailInputs {
                tools: vec![tool],
                texts,
            },
            description,
            schema,
            schema_leaves,
            argument_leaves,
        }))
    }
}

pub(super) fn argument_leaves(arguments: Option<&McpValue>) -> Result<Vec<Leaf>, ContentBlocked> {
    match arguments {
        Some(value) => Ok(scan_json(value, NestedPayload::ToolArguments)?.leaves),
        None => Ok(Vec::new()),
    }
}

// the same defaults LiteLLM's MCP-to-OpenAI tool conversion fills in for function parameters
fn tool_definition(
    name: &str,
    description: Option<&str>,
    schema: &BTreeMap<String, McpValue>,
) -> ToolDefinition {
    let mut parameters = schema.clone();
    parameters
        .entry("type".to_owned())
        .or_insert_with(|| McpValue::Text("object".to_owned()));
    parameters
        .entry("properties".to_owned())
        .or_insert_with(|| McpValue::Object(BTreeMap::new()));
    parameters
        .entry("additionalProperties".to_owned())
        .or_insert(McpValue::Boolean(false));
    ToolDefinition {
        kind: "function",
        function: FunctionDefinition {
            name: name.to_owned(),
            description: description.unwrap_or_default().to_owned(),
            parameters,
            strict: false,
        },
    }
}

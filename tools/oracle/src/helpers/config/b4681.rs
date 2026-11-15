use betterlite_config::sections::mcp_toolset_storage::table::ToolsetRow;
use betterlite_config::sections::mcp_toolset_storage::{
    MCPToolset, MCPToolsetStorageError, MCPToolsetTool, NewMCPToolsetRequest,
    UpdateMCPToolsetRequest, create_mcp_toolset, delete_mcp_toolset, get_mcp_toolset,
    get_mcp_toolset_by_name, list_mcp_toolsets, update_mcp_toolset,
};
use chrono::NaiveDateTime;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;
use crate::helpers::{Helper, HelperError, HelperResult};

mod store;

pub(crate) struct Op;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Create,
    Get,
    GetByName,
    List,
    Update,
    Delete,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    timestamp: NaiveDateTime,
    #[serde(default)]
    touched_by: String,
    rows: Vec<RowInput>,
    #[serde(default)]
    request: Option<Value>,
    #[serde(default)]
    toolset_id: Option<String>,
    #[serde(default)]
    toolset_name: Option<String>,
    #[serde(default)]
    toolset_ids: Option<Vec<String>>,
    #[serde(default)]
    use_writer: bool,
    #[serde(default)]
    fail: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RowInput {
    toolset_id: String,
    toolset_name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tools: Option<Vec<MCPToolsetTool>>,
}

impl RowInput {
    fn to_row(&self, timestamp: NaiveDateTime) -> Result<ToolsetRow, Box<HelperError>> {
        let tools = match &self.tools {
            Some(tools) => Some(serde_json::to_string(tools).or_value_error()?),
            None => None,
        };
        Ok(ToolsetRow {
            toolset_id: self.toolset_id.clone(),
            toolset_name: self.toolset_name.clone(),
            description: self.description.clone(),
            tools,
            created_at: Some(timestamp),
            created_by: None,
            updated_at: Some(timestamp),
            updated_by: None,
        })
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let rows = input
            .rows
            .iter()
            .map(|row| row.to_row(input.timestamp))
            .collect::<Result<Vec<ToolsetRow>, Box<HelperError>>>()?;
        let store = store::Store::new(rows, input.timestamp, input.fail);
        runtime.block_on(call(input, &store))
    }
}

async fn call(input: &Input, store: &store::Store) -> HelperResult {
    match input.operation {
        Operation::Create => create(input, store).await,
        Operation::Get => get(input, store).await,
        Operation::GetByName => get_by_name(input, store).await,
        Operation::List => list(input, store).await,
        Operation::Update => update(input, store).await,
        Operation::Delete => delete(input, store).await,
    }
}

async fn create(input: &Input, store: &store::Store) -> HelperResult {
    let request: NewMCPToolsetRequest = request(input)?;
    let created = create_mcp_toolset(store, &request, &input.touched_by)
        .await
        .map_err(|error| storage_error(&error))?;
    serde_json::to_value(created).or_value_error()
}

async fn get(input: &Input, store: &store::Store) -> HelperResult {
    let toolset_id = required(input.toolset_id.as_deref())?;
    let found = get_mcp_toolset(store, toolset_id)
        .await
        .map_err(|error| storage_error(&error))?;
    optional(found)
}

async fn get_by_name(input: &Input, store: &store::Store) -> HelperResult {
    let toolset_name = required(input.toolset_name.as_deref())?;
    let found = get_mcp_toolset_by_name(store, toolset_name)
        .await
        .map_err(|error| storage_error(&error))?;
    optional(found)
}

async fn list(input: &Input, store: &store::Store) -> HelperResult {
    let toolsets = list_mcp_toolsets(store, input.toolset_ids.as_deref(), input.use_writer)
        .await
        .map_err(|error| storage_error(&error))?;
    Ok(json!({"toolsets": toolsets, "store": store.seen()}))
}

async fn update(input: &Input, store: &store::Store) -> HelperResult {
    let request: UpdateMCPToolsetRequest = request(input)?;
    let updated = update_mcp_toolset(store, &request, &input.touched_by)
        .await
        .map_err(|error| storage_error(&error))?;
    optional(updated)
}

async fn delete(input: &Input, store: &store::Store) -> HelperResult {
    let toolset_id = required(input.toolset_id.as_deref())?;
    let removed = delete_mcp_toolset(store, toolset_id)
        .await
        .map_err(|error| storage_error(&error))?;
    optional(removed)
}

fn request<T>(input: &Input) -> Result<T, Box<HelperError>>
where
    T: DeserializeOwned,
{
    let value = input
        .request
        .as_ref()
        .ok_or_else(|| value_error("the case names no request"))?;
    serde_json::from_value(value.clone()).or_value_error()
}

fn required(value: Option<&str>) -> Result<&str, Box<HelperError>> {
    let Some(name) = value else {
        return Err(value_error("the case names no toolset"));
    };
    Ok(name)
}

fn optional(found: Option<MCPToolset>) -> HelperResult {
    found.map_or_else(
        || Ok(Value::Null),
        |toolset| serde_json::to_value(toolset).or_value_error(),
    )
}

fn storage_error(error: &MCPToolsetStorageError) -> Box<HelperError> {
    let message = error.to_string();
    Box::new(HelperError {
        class: "DatabaseUnavailableError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

fn value_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_owned(),
    })
}

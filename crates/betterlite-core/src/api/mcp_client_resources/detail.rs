use betterlite_mcp::client::sessions::client::SessionClient;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::runtime::McpRuntime;

use super::operations::run_detail_operation;
use super::session::{ResourceSession, RuntimeSession, session_error};
use super::types::{GetPromptRequestParams, GetPromptResult, ReadResourceResult};

pub async fn get_prompt<Runtime>(
    client: &mut SessionClient<Runtime>,
    params: GetPromptRequestParams,
) -> Result<GetPromptResult, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
{
    let operation =
        async move |session: &mut RuntimeSession<Runtime>| match session.get_prompt(params).await {
            Ok(result) => Ok(result),
            Err(error) => Err(session_error(&*session, error)),
        };
    run_detail_operation(client, "get_prompt", operation).await
}

pub async fn read_resource<Runtime>(
    client: &mut SessionClient<Runtime>,
    uri: String,
) -> Result<ReadResourceResult, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
{
    let operation =
        async move |session: &mut RuntimeSession<Runtime>| match session.read_resource(uri).await {
            Ok(result) => Ok(result),
            Err(error) => Err(session_error(&*session, error)),
        };
    run_detail_operation(client, "read_resource", operation).await
}

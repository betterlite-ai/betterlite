use betterlite_mcp::client::sessions::client::SessionClient;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::runtime::McpRuntime;
use url::Url;

use super::operations::run_detail_operation;
use super::session::{ResourceSession, RuntimeSession, session_error};
use super::types::{GetPromptRequestParams, GetPromptResult, ReadResourceResult};

macro_rules! detail_operation {
    ($method:ident, $request:ty, $output:ty, $call:ident) => {
        pub async fn $method<Runtime>(
            client: &mut SessionClient<Runtime>,
            request: $request,
        ) -> Result<$output, SessionError<Runtime::Error>>
        where
            Runtime: McpRuntime,
            RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
        {
            let operation = async move |session: &mut RuntimeSession<Runtime>| match session
                .$call(request)
                .await
            {
                Ok(result) => Ok(result),
                Err(error) => Err(session_error(&*session, error)),
            };
            run_detail_operation(client, stringify!($method), operation).await
        }
    };
}

detail_operation!(
    get_prompt,
    GetPromptRequestParams,
    GetPromptResult,
    get_prompt
);
detail_operation!(read_resource, Url, ReadResourceResult, read_resource);

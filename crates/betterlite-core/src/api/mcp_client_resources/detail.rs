use betterlite_mcp::client::sessions::client::SessionClient;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::runtime::McpRuntime;
use url::Url;

use super::operations::run_detail_operation;
use super::session::{ResourceSession, RuntimeSession, session_error};
use super::types::{GetPromptRequestParams, GetPromptResult, ReadResourceResult};

macro_rules! detail_operations {
    ($($method:ident, $from_session:ident, $request:ty, $output:ty, $call:ident;)*) => {
        $(
            pub async fn $from_session<Session>(
                session: &mut Session,
                request: $request,
            ) -> Result<$output, SessionError<Session::Error>>
            where
                Session: ResourceSession,
            {
                match session.$call(request).await {
                    Ok(result) => Ok(result),
                    Err(error) => Err(session_error(session, error)),
                }
            }

            pub async fn $method<Runtime>(
                client: &mut SessionClient<Runtime>,
                request: $request,
            ) -> Result<$output, SessionError<Runtime::Error>>
            where
                Runtime: McpRuntime,
                RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
            {
                let operation = async move |session: &mut RuntimeSession<Runtime>| {
                    $from_session(session, request).await
                };
                run_detail_operation(client, stringify!($method), operation).await
            }
        )*
    };
}

detail_operations!(
    get_prompt, get_prompt_from_session, GetPromptRequestParams, GetPromptResult, get_prompt;
    read_resource, read_resource_from_session, Url, ReadResourceResult, read_resource;
);

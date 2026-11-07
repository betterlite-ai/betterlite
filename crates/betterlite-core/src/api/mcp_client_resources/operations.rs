use std::future::Future;
use std::ops::AsyncFnOnce;

use betterlite_mcp::client::sessions::client::SessionClient;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::run::run_with_session;
use betterlite_mcp::client::sessions::runtime::McpRuntime;
use tracing::info;

use super::detail::{get_prompt, read_resource};
use super::failure::{FailureMode, finish_failure};
use super::list::{Prompts, Resources, Templates, run_optional_list};
use super::session::{ResourceSession, RuntimeSession};
use super::settings::ResourceSettings;
use super::types::{
    GetPromptRequestParams, GetPromptResult, Prompt, ReadResourceResult, Resource, ResourceTemplate,
};

macro_rules! resource_operations {
    ($($method:ident: $kind:ty => $item:ty),* $(,)?) => {
        pub trait McpResourceOperations<Runtime>
        where
            Runtime: McpRuntime,
            RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
        {
            $(
                fn $method(
                    &mut self,
                    settings: &ResourceSettings,
                    raise_on_error: bool,
                ) -> impl Future<Output = Result<Vec<$item>, SessionError<Runtime::Error>>>;
            )*

            fn get_prompt(
                &mut self,
                params: GetPromptRequestParams,
            ) -> impl Future<Output = Result<GetPromptResult, SessionError<Runtime::Error>>>;

            fn read_resource(
                &mut self,
                uri: String,
            ) -> impl Future<Output = Result<ReadResourceResult, SessionError<Runtime::Error>>>;
        }

        impl<Runtime> McpResourceOperations<Runtime> for SessionClient<Runtime>
        where
            Runtime: McpRuntime,
            RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
        {
            $(
                async fn $method(
                    &mut self,
                    settings: &ResourceSettings,
                    raise_on_error: bool,
                ) -> Result<Vec<$item>, SessionError<Runtime::Error>> {
                    run_optional_list::<Runtime, $kind>(self, settings, raise_on_error).await
                }
            )*

            async fn get_prompt(
                &mut self,
                params: GetPromptRequestParams,
            ) -> Result<GetPromptResult, SessionError<Runtime::Error>> {
                get_prompt(self, params).await
            }

            async fn read_resource(
                &mut self,
                uri: String,
            ) -> Result<ReadResourceResult, SessionError<Runtime::Error>> {
                read_resource(self, uri).await
            }
        }
    };
}

resource_operations!(
    list_prompts: Prompts => Prompt,
    list_resources: Resources => Resource,
    list_resource_templates: Templates => ResourceTemplate,
);

pub(crate) async fn run_detail_operation<Runtime, Output, Operation>(
    client: &mut SessionClient<Runtime>,
    operation_name: &str,
    operation: Operation,
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
    Output: Default,
    Operation:
        AsyncFnOnce(&mut RuntimeSession<Runtime>) -> Result<Output, SessionError<Runtime::Error>>,
{
    let server = server_name(client);
    match run_with_session(client, operation, false).await {
        Ok(output) => {
            info!("MCP client {operation_name} completed successfully from {server}");
            Ok(output)
        }
        Err(error) => finish_failure(error, FailureMode::Detail, operation_name, &server),
    }
}

pub(crate) fn server_name<Runtime>(client: &SessionClient<Runtime>) -> String
where
    Runtime: McpRuntime,
{
    if client.auth.server_url.is_empty() {
        "stdio".to_owned()
    } else {
        client.auth.server_url.clone()
    }
}

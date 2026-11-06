use std::future::Future;
use std::ops::AsyncFnOnce;

use betterlite_mcp::client::sessions::client::SessionClient;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::run::run_with_session;
use betterlite_mcp::client::sessions::runtime::McpRuntime;
use tracing::info;

use super::failure::detail_failure;
use super::get_prompt::get_prompt;
use super::list::{Prompts, Resources, Templates, run_optional_list};
use super::read_resource::read_resource;
use super::session::{ResourceSession, RuntimeSession};
use super::settings::ResourceSettings;
use super::types::{
    GetPromptRequestParams, GetPromptResult, Prompt, ReadResourceResult, Resource, ResourceTemplate,
};

pub trait McpResourceOperations<Runtime>
where
    Runtime: McpRuntime,
    RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
{
    fn list_prompts(
        &mut self,
        settings: &ResourceSettings,
        raise_on_error: bool,
    ) -> impl Future<Output = Result<Vec<Prompt>, SessionError<Runtime::Error>>>;

    fn list_resources(
        &mut self,
        settings: &ResourceSettings,
        raise_on_error: bool,
    ) -> impl Future<Output = Result<Vec<Resource>, SessionError<Runtime::Error>>>;

    fn list_resource_templates(
        &mut self,
        settings: &ResourceSettings,
        raise_on_error: bool,
    ) -> impl Future<Output = Result<Vec<ResourceTemplate>, SessionError<Runtime::Error>>>;

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
    async fn list_prompts(
        &mut self,
        settings: &ResourceSettings,
        raise_on_error: bool,
    ) -> Result<Vec<Prompt>, SessionError<Runtime::Error>> {
        run_optional_list::<Runtime, Prompts>(self, settings, raise_on_error).await
    }

    async fn list_resources(
        &mut self,
        settings: &ResourceSettings,
        raise_on_error: bool,
    ) -> Result<Vec<Resource>, SessionError<Runtime::Error>> {
        run_optional_list::<Runtime, Resources>(self, settings, raise_on_error).await
    }

    async fn list_resource_templates(
        &mut self,
        settings: &ResourceSettings,
        raise_on_error: bool,
    ) -> Result<Vec<ResourceTemplate>, SessionError<Runtime::Error>> {
        run_optional_list::<Runtime, Templates>(self, settings, raise_on_error).await
    }

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

pub(crate) async fn run_detail_operation<Runtime, Output, Operation>(
    client: &mut SessionClient<Runtime>,
    operation_name: &str,
    operation: Operation,
) -> Result<Output, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
    Operation:
        AsyncFnOnce(&mut RuntimeSession<Runtime>) -> Result<Output, SessionError<Runtime::Error>>,
{
    let server = server_name(client);
    match run_with_session(client, operation, false).await {
        Ok(output) => {
            info!("MCP client {operation_name} completed successfully from {server}");
            Ok(output)
        }
        Err(error) => detail_failure(error, operation_name, &server),
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

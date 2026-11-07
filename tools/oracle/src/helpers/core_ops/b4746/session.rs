use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_core::api::mcp_client_resources::session::ResourceSession;
use betterlite_core::api::mcp_client_resources::types::{
    GetPromptRequestParams, GetPromptResult, PromptPage, ReadResourceResult, ResourcePage,
    ResourceTemplatePage, ServerCapabilities,
};
use betterlite_mcp::client::sessions::error::McpFailure;
use thiserror::Error;
use tokio::time::sleep;

use super::input::PageInput;

#[derive(Debug, Error)]
pub(crate) enum ScriptedFailure {
    #[error("MCP error {0:?}")]
    Protocol(McpFailure),
    #[error("{0}")]
    Plain(String),
}

impl ScriptedFailure {
    fn protocol_failure(&self) -> Option<McpFailure> {
        match self {
            Self::Protocol(failure) => Some(failure.clone()),
            Self::Plain(_) => None,
        }
    }
}

pub(crate) enum Step<Value> {
    Value {
        value: Value,
        delay: Duration,
    },
    Failure {
        failure: McpFailure,
        delay: Duration,
    },
}

pub(crate) struct Script {
    pub(crate) capabilities: Option<ServerCapabilities>,
    pub(crate) pages: VecDeque<PageInput>,
    pub(crate) prompt_result: Option<Step<GetPromptResult>>,
    pub(crate) resource_result: Option<Step<ReadResourceResult>>,
    pub(crate) cursors: Vec<Option<String>>,
    pub(crate) prompt_params: Vec<GetPromptRequestParams>,
    pub(crate) resource_uris: Vec<String>,
}

impl Script {
    pub(crate) const fn new(capabilities: Option<ServerCapabilities>) -> Self {
        Self {
            capabilities,
            pages: VecDeque::new(),
            prompt_result: None,
            resource_result: None,
            cursors: Vec::new(),
            prompt_params: Vec::new(),
            resource_uris: Vec::new(),
        }
    }
}

pub(crate) trait Locked<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) struct FakeSession {
    pub(crate) script: Arc<Mutex<Script>>,
}

impl FakeSession {
    async fn next_page(&self, cursor: Option<String>) -> Result<PageInput, ScriptedFailure> {
        let page = {
            let mut script = self.script.locked();
            script.cursors.push(cursor);
            script.pages.pop_front().ok_or_else(|| {
                ScriptedFailure::Plain("the case scripted no page for this call".to_owned())
            })?
        };
        sleep(Duration::from_millis(page.delay_ms)).await;
        if let Some(error) = &page.error {
            return Err(ScriptedFailure::Protocol(McpFailure {
                code: error.code,
                message: error.message.clone(),
                context: None,
            }));
        }
        Ok(page)
    }
}

async fn resolve<Value>(step: Step<Value>) -> Result<Value, ScriptedFailure> {
    match step {
        Step::Value { value, delay } => {
            sleep(delay).await;
            Ok(value)
        }
        Step::Failure { failure, delay } => {
            sleep(delay).await;
            Err(ScriptedFailure::Protocol(failure))
        }
    }
}

fn items<Value>(values: Vec<serde_json::Value>) -> Result<Vec<Value>, ScriptedFailure>
where
    Value: serde::de::DeserializeOwned,
{
    values
        .into_iter()
        .map(|value| serde_json::from_value::<Value>(value))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ScriptedFailure::Plain(error.to_string()))
}

impl ResourceSession for FakeSession {
    type Error = ScriptedFailure;

    fn server_capabilities(&self) -> Option<ServerCapabilities> {
        self.script.locked().capabilities.clone()
    }

    async fn list_prompts(
        &mut self,
        cursor: Option<String>,
    ) -> Result<PromptPage, ScriptedFailure> {
        let page = self.next_page(cursor).await?;
        Ok(PromptPage {
            prompts: items(page.items)?,
            next_cursor: page.next_cursor,
        })
    }

    async fn list_resources(
        &mut self,
        cursor: Option<String>,
    ) -> Result<ResourcePage, ScriptedFailure> {
        let page = self.next_page(cursor).await?;
        Ok(ResourcePage {
            resources: items(page.items)?,
            next_cursor: page.next_cursor,
        })
    }

    async fn list_resource_templates(
        &mut self,
        cursor: Option<String>,
    ) -> Result<ResourceTemplatePage, ScriptedFailure> {
        let page = self.next_page(cursor).await?;
        Ok(ResourceTemplatePage {
            resource_templates: items(page.items)?,
            next_cursor: page.next_cursor,
        })
    }

    async fn get_prompt(
        &mut self,
        params: GetPromptRequestParams,
    ) -> Result<GetPromptResult, ScriptedFailure> {
        let scripted = {
            let mut script = self.script.locked();
            script.prompt_params.push(params);
            script.prompt_result.take().ok_or_else(|| {
                ScriptedFailure::Plain("the case scripted no result for this call".to_owned())
            })
        };
        resolve(scripted?).await
    }

    async fn read_resource(&mut self, uri: String) -> Result<ReadResourceResult, ScriptedFailure> {
        let scripted = {
            let mut script = self.script.locked();
            script.resource_uris.push(uri);
            script.resource_result.take().ok_or_else(|| {
                ScriptedFailure::Plain("the case scripted no result for this call".to_owned())
            })
        };
        resolve(scripted?).await
    }

    fn protocol_failure(error: &ScriptedFailure) -> Option<McpFailure> {
        error.protocol_failure()
    }
}

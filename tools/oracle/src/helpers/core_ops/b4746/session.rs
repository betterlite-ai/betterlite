use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::mcp_client_resources::session::ResourceSession;
use betterlite_core::api::mcp_client_resources::types::{
    GetPromptRequestParams, GetPromptResult, PromptPage, ReadResourceResult, ResourcePage,
    ResourceTemplatePage, ServerCapabilities,
};
use betterlite_mcp::client::sessions::error::McpFailure;
use thiserror::Error;
use tokio::time::sleep;
use url::Url;

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

pub(crate) type ResultStep<Value> = Result<Value, McpFailure>;

pub(crate) struct Script {
    pub(crate) capabilities: Option<ServerCapabilities>,
    pub(crate) pages: VecDeque<PageInput>,
    pub(crate) prompt_result: Option<ResultStep<GetPromptResult>>,
    pub(crate) resource_result: Option<ResultStep<ReadResourceResult>>,
    pub(crate) cursors: Vec<Option<String>>,
    pub(crate) prompt_params: Vec<GetPromptRequestParams>,
    pub(crate) resource_uris: Vec<Url>,
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

pub(crate) struct FakeSession {
    pub(crate) script: Arc<Mutex<Script>>,
}

impl FakeSession {
    async fn next_page(&self, cursor: Option<String>) -> Result<PageInput, ScriptedFailure> {
        let page = {
            let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
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

macro_rules! page_method {
    ($method:ident, $page:ident, $field:ident) => {
        async fn $method(&mut self, cursor: Option<String>) -> Result<$page, ScriptedFailure> {
            let page = self.next_page(cursor).await?;
            let items = page
                .items
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| ScriptedFailure::Plain(error.to_string()))?;
            Ok($page {
                $field: items,
                next_cursor: page.next_cursor,
            })
        }
    };
}

macro_rules! result_method {
    ($method:ident, $queue:ident, $record:ident, $request:ty, $output:ty) => {
        fn $method(
            &mut self,
            request: $request,
        ) -> impl Future<Output = Result<$output, ScriptedFailure>> + Send {
            let scripted = {
                let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
                script.$record.push(request);
                script.$queue.take().ok_or_else(|| {
                    ScriptedFailure::Plain("the case scripted no result for this call".to_owned())
                })
            };
            ready(match scripted {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(failure)) => Err(ScriptedFailure::Protocol(failure)),
                Err(error) => Err(error),
            })
        }
    };
}

impl ResourceSession for FakeSession {
    type Error = ScriptedFailure;

    fn server_capabilities(&self) -> Option<ServerCapabilities> {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .capabilities
            .clone()
    }

    page_method!(list_prompts, PromptPage, prompts);
    page_method!(list_resources, ResourcePage, resources);
    page_method!(
        list_resource_templates,
        ResourceTemplatePage,
        resource_templates
    );
    result_method!(
        get_prompt,
        prompt_result,
        prompt_params,
        GetPromptRequestParams,
        GetPromptResult
    );
    result_method!(
        read_resource,
        resource_result,
        resource_uris,
        Url,
        ReadResourceResult
    );

    fn protocol_failure(error: &ScriptedFailure) -> Option<McpFailure> {
        error.protocol_failure()
    }
}

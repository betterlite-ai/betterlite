pub mod call;
pub mod error;
pub mod request;
mod resolution;
pub mod sdk_dispatch;
mod tests;

use std::error::Error;
use std::future::Future;

use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::records::{AssistantDeleted, AssistantObject, AssistantPage};
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;

use crate::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, ItemsBoundary, JsonBoundary,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use call::{AssistantCall, AssistantProviderCall};
use error::AssistantReadDeleteError;
use request::{AssistantOperation, AssistantReadDeleteInput, AssistantWrapper};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantReply {
    Page(AssistantPage),
    Deleted(AssistantDeleted),
    Assistant(Box<AssistantObject>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantStep {
    Page(AssistantPage),
    Deleted(AssistantDeleted),
    Coroutine,
}

#[derive(Debug, Clone)]
pub struct AssistantReadDeleteResult {
    pub call: AssistantCall,
    pub step: AssistantStep,
}

pub trait AssistantReadDeleteDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn call(
        &self,
        call: &AssistantCall,
    ) -> impl Future<Output = Result<Option<AssistantReply>, Self::Error>> + Send;
}

pub struct AssistantReadDelete<Dispatch> {
    pub dispatch: Dispatch,
    pub limits: ApiLimits,
    pub settings: Settings,
}

impl<Dispatch> AssistantReadDelete<Dispatch>
where
    Dispatch: AssistantReadDeleteDispatch,
    Dispatch::Error: Error + Send + Sync + 'static,
{
    pub async fn run(
        &self,
        input: &AssistantReadDeleteInput,
    ) -> Result<AssistantReadDeleteResult, AssistantReadDeleteError<Dispatch::Error>> {
        check_limits(&self.limits, input)?;
        let flag = requested_flag(input)
            .map_err(|(flag, class)| AssistantReadDeleteError::InvalidFlag { flag, class })?;
        let (call, asynchronous_client) = self.resolve(input, flag)?;
        if input.entry == AssistantWrapper::Sync && asynchronous_client {
            return Ok(AssistantReadDeleteResult {
                call,
                step: AssistantStep::Coroutine,
            });
        }
        let reply = self
            .dispatch
            .call(&call)
            .await
            .map_err(AssistantReadDeleteError::Dispatch)?;
        let Some(answer) = reply else {
            // a null list reply repeats the get_assistants bad request, while delete answers 500
            return Err(match input.operation {
                AssistantOperation::List => AssistantReadDeleteError::Unsupported {
                    provider: input.provider.clone(),
                    function: input.operation.function(),
                },
                AssistantOperation::Delete => AssistantReadDeleteError::MissingReply {
                    function: input.operation.function(),
                },
            });
        };
        let step = match (input.operation, answer) {
            (AssistantOperation::List, AssistantReply::Page(page)) => AssistantStep::Page(page),
            (AssistantOperation::Delete, AssistantReply::Deleted(deleted)) => {
                AssistantStep::Deleted(deleted)
            }
            (_, _) => return Err(AssistantReadDeleteError::MismatchedReply),
        };
        Ok(AssistantReadDeleteResult { call, step })
    }

    fn resolve(
        &self,
        input: &AssistantReadDeleteInput,
        flag: Option<bool>,
    ) -> Result<(AssistantCall, bool), AssistantReadDeleteError<Dispatch::Error>> {
        if input.operation == AssistantOperation::Delete && input.assistant_id.is_none() {
            return Err(AssistantReadDeleteError::MissingAssistantId);
        }
        let timeout = resolution::resolve_timeout(input)?;
        let settings = ResourceSdkSettings::from(&self.settings);
        let options = ResourceSdkRequestOptions::from(&input.params);
        // the flag only picks the client type; a supplied async client makes the call async on its own
        let kind = match flag {
            Some(true) => ClientKind::Asynchronous,
            Some(false) | None => ClientKind::Sync,
        };
        let provider = match input.provider.as_str() {
            "openai" => AssistantProviderCall::OpenAi(Box::new(resolution::openai(
                input, &settings, &options, timeout,
            ))),
            "azure" => AssistantProviderCall::Azure(Box::new(resolution::azure(
                input, &settings, &options, timeout,
            ))),
            _ => {
                return Err(AssistantReadDeleteError::Unsupported {
                    provider: input.provider.clone(),
                    function: input.operation.function(),
                });
            }
        };
        let call = AssistantCall {
            operation: input.operation,
            provider,
            assistant_id: input.assistant_id.clone(),
            kind,
            supplied: resolution::supplied_client(input),
            timeout,
        };
        let asynchronous = call.kind == ClientKind::Asynchronous
            || call
                .supplied
                .as_ref()
                .is_some_and(|client| client.kind.client_kind() == ClientKind::Asynchronous);
        Ok((call, asynchronous))
    }
}

// Only the sync entry sees the caller's flag: each async wrapper overwrites it with True.
fn requested_flag(
    input: &AssistantReadDeleteInput,
) -> Result<Option<bool>, (&'static str, &'static str)> {
    if input.entry == AssistantWrapper::Async {
        return Ok(Some(true));
    }
    let raw = match input.operation {
        AssistantOperation::List => input.aget_assistants.as_ref(),
        AssistantOperation::Delete => input.async_delete_assistants.as_ref(),
    };
    match raw {
        None => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        // the flag's name, and the class its sync function raises when the value isn't a bool
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(match input.operation {
                AssistantOperation::List => ("aget_assistants", "Exception"),
                AssistantOperation::Delete => ("async_delete_assistants", "ValueError"),
            })
        }
    }
}

fn check_limits(limits: &ApiLimits, input: &AssistantReadDeleteInput) -> Result<(), ApiLimitError> {
    let body = input.extra_body.as_ref().and_then(Option::as_ref);
    let known_count = body.map_or(0, IndexMap::len);
    limits.check(ApiBoundariesInput::Items(ItemsBoundary { known_count }))?;
    let body_bytes = body.map_or(0, |fields| {
        serde_json::to_vec(fields).map_or(usize::MAX, |bytes| bytes.len())
    });
    let id_bytes = input.assistant_id.as_ref().map_or(0, String::len);
    let known_bytes = id_bytes.saturating_add(body_bytes);
    limits.check(ApiBoundariesInput::Json(JsonBoundary { known_bytes }))?;
    Ok(())
}

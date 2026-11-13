use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::mem::take;
use std::sync::Mutex;

use betterlite_error::error::details::{RateLimitDetails, ResponseDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_openai::search::records::SearchResponse;
use betterlite_websearch::call_transformation::outcome::SearchFailure;
use betterlite_websearch::execution::WebSearchExecution;
use betterlite_websearch::execution::capabilities::ProviderCapabilities;
use betterlite_websearch::execution::runner::{SearchCall, SearchRunner};
use betterlite_websearch::execution::search::unclassified;
use betterlite_websearch::search_tool_access::authorization::{
    SearchToolAuthorizer, SearchToolGrants,
};
use betterlite_websearch::search_tool_access::hook_kwargs::{ParentOtelSpan, SearchCaller};
use betterlite_websearch::search_tool_access::search_metadata::{
    SearchMetadataSource, SearchRequestMetadata,
};
use betterlite_websearch::search_tool_access::tool_selection::SearchToolParams;
use serde::Serialize;

use super::{ErrorKind, Input, ScriptedError};

pub(super) type ScriptedExecution =
    WebSearchExecution<ScriptedRunner, ScriptedGrants, ScriptedMetadata, ScriptedCapabilities>;

#[derive(Serialize)]
pub(super) struct RecordedCall {
    pub(super) query: SearchQuery,
    pub(super) search_provider: String,
    pub(super) params: SearchToolParams,
    pub(super) metadata: Option<SearchRequestMetadata>,
}

pub(super) struct ScriptedRunner {
    pub(super) outcome: Result<SearchResponse, SearchFailure>,
    pub(super) calls: Mutex<Vec<RecordedCall>>,
}

impl SearchRunner for ScriptedRunner {
    fn search(
        &self,
        call: SearchCall<'_>,
    ) -> impl Future<Output = Result<SearchResponse, SearchFailure>> + Send {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(RecordedCall {
                query: call.query,
                search_provider: call.search_provider.to_owned(),
                params: call.params.clone(),
                metadata: call.metadata.cloned(),
            });
        }
        ready(self.outcome.clone())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("search tool access denied")]
pub(super) struct GrantDenied;

pub(super) struct ScriptedGrants {
    pub(super) denied: bool,
}

impl SearchToolGrants for ScriptedGrants {
    type Team = ();
    type Error = GrantDenied;

    fn check_key(
        &self,
        _name: &str,
        _caller: &SearchCaller,
    ) -> impl Future<Output = Result<(), GrantDenied>> + Send {
        let outcome = if self.denied {
            Err(GrantDenied)
        } else {
            Ok(())
        };
        ready(outcome)
    }

    fn load_team(
        &self,
        _team_id: &str,
        _parent_otel_span: Option<&ParentOtelSpan>,
    ) -> impl Future<Output = Result<(), GrantDenied>> + Send {
        ready(Ok(()))
    }

    fn check_team(
        &self,
        _name: &str,
        _team: &(),
    ) -> impl Future<Output = Result<(), GrantDenied>> + Send {
        ready(Ok(()))
    }
}

pub(super) struct ScriptedMetadata;

impl SearchMetadataSource for ScriptedMetadata {
    fn sanitized_key_metadata(&self, _caller: &SearchCaller) -> SearchRequestMetadata {
        SearchRequestMetadata::new()
    }

    fn logged_api_key(&self, _caller: &SearchCaller) -> Option<String> {
        None
    }
}

pub(super) struct ScriptedCapabilities {
    pub(super) native_providers: Vec<String>,
    pub(super) rich_providers: Vec<String>,
}

impl ProviderCapabilities for ScriptedCapabilities {
    fn handles_web_search_natively(&self, _model: &str, provider: &str) -> bool {
        self.native_providers.iter().any(|name| name == provider)
    }

    fn supports_rich_search_input(&self, search_provider: &str) -> bool {
        self.rich_providers
            .iter()
            .any(|name| name == search_provider)
    }
}

pub(super) fn build(input: &Input) -> ScriptedExecution {
    let runner = ScriptedRunner {
        outcome: scripted_outcome(input.search_error.as_ref(), input.search_response.as_ref()),
        calls: Mutex::new(Vec::new()),
    };
    WebSearchExecution {
        enabled_providers: input.enabled_providers.clone(),
        search_tool_name: input.search_tool_name.clone(),
        search_tools: input.search_tools.clone(),
        runner,
        authorizer: SearchToolAuthorizer {
            grants_port: ScriptedGrants {
                denied: input.deny_search_tool,
            },
        },
        metadata_source: ScriptedMetadata,
        capabilities: ScriptedCapabilities {
            native_providers: input.native_providers.clone(),
            rich_providers: input.rich_providers.clone(),
        },
    }
}

pub(super) fn recorded(execution: &ScriptedExecution) -> Vec<RecordedCall> {
    execution
        .runner
        .calls
        .lock()
        .map_or_else(|_| Vec::new(), |mut calls| take(&mut *calls))
}

fn scripted_outcome(
    error: Option<&ScriptedError>,
    response: Option<&SearchResponse>,
) -> Result<SearchResponse, SearchFailure> {
    error.map_or_else(
        || {
            response.map_or_else(
                || {
                    Err(unclassified(
                        "the case scripts no search response".to_owned(),
                    ))
                },
                |scripted| Ok(scripted.clone()),
            )
        },
        |scripted| Err(scripted.failure()),
    )
}

impl ScriptedError {
    fn failure(&self) -> SearchFailure {
        let context = ErrorContext {
            message: self.message.clone(),
            model: None,
            provider: None,
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        };
        let class = match &self.kind {
            ErrorKind::RateLimit => ErrorClass::RateLimit(Box::new(RateLimitDetails {
                context,
                category: RateLimitCategory::VendorRateLimit,
                dimension: None,
                headers: BTreeMap::new(),
            })),
            ErrorKind::ContextWindow => {
                ErrorClass::ContextWindowExceeded(Box::new(ResponseDetails::from(context)))
            }
            ErrorKind::Unavailable => ErrorClass::Unclassified(Box::new(context)),
        };
        SearchFailure {
            class,
            message: self.message.clone(),
        }
    }
}

use betterlite_websearch::search_tool_access::hook_kwargs::{
    SearchCaller, SearchHookKwargs, caller_from_kwargs,
};
use betterlite_websearch::search_tool_access::parent_correlation::{
    ParentRequestCorrelation, parent_correlation,
};
use betterlite_websearch::search_tool_access::search_metadata::SearchRequestMetadata;
use betterlite_websearch::search_tool_access::tool_selection::{
    SearchToolConfig, SearchToolSelector,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    SelectTool,
    CallerFromKwargs,
    ParentCorrelation,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    search_tool_name: Option<String>,
    #[serde(default)]
    search_tools: Option<Vec<SearchToolConfig>>,
    #[serde(default)]
    kwargs: Option<SearchHookKwargs>,
    #[serde(default)]
    caller: Option<SearchCaller>,
}

#[derive(Serialize)]
struct SelectedTool {
    search_tool_name: Option<String>,
    search_provider: Option<String>,
    litellm_params: IndexMap<String, Box<RawValue>>,
}

impl From<&SearchToolConfig> for SelectedTool {
    fn from(config: &SearchToolConfig) -> Self {
        Self {
            search_tool_name: config.search_tool_name.clone(),
            search_provider: config.search_provider.clone(),
            litellm_params: config.litellm_params.clone(),
        }
    }
}

#[derive(Serialize)]
struct SelectionOutput {
    tool: Option<SelectedTool>,
}

#[derive(Serialize)]
struct CorrelationOutput {
    correlation: ParentRequestCorrelation,
    metadata: SearchRequestMetadata,
    kwargs: IndexMap<String, String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation {
            Operation::SelectTool => select_tool(input),
            Operation::CallerFromKwargs => caller_value(input),
            Operation::ParentCorrelation => correlation_value(input),
        }
    }
}

fn select_tool(input: &Input) -> crate::helpers::HelperResult {
    let tool = SearchToolSelector::new(input.search_tool_name.clone())
        .select_from_router(input.search_tools.as_deref())
        .map(SelectedTool::from);
    serde_json::to_value(SelectionOutput { tool }).map_err(failure)
}

fn caller_value(input: &Input) -> crate::helpers::HelperResult {
    let Some(caller) = caller_from_kwargs(input.kwargs.as_ref()) else {
        return Ok(serde_json::Value::Null);
    };
    serde_json::to_value(caller).map_err(failure)
}

fn correlation_value(input: &Input) -> crate::helpers::HelperResult {
    let correlation = parent_correlation(input.kwargs.as_ref(), input.caller.as_ref());
    let metadata = correlation.as_search_metadata();
    let kwargs = correlation.as_search_kwargs();
    serde_json::to_value(CorrelationOutput {
        correlation,
        metadata,
        kwargs,
    })
    .map_err(failure)
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "SearchToolAccessError".to_owned(),
        message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

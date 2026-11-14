use betterlite_openai::search::records::SearchResponse;
use betterlite_prompts::message_content::Message;
use betterlite_websearch::search_tool_access::hook_kwargs::SearchHookKwargs;
use betterlite_websearch::search_tool_access::tool_selection::SearchToolConfig;
use betterlite_websearch::tools::Tool;
use serde::Deserialize;
use serde_json::value::RawValue;

mod fakes;
mod operations;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    ShortCircuit,
    ExecuteSearch,
    RichSearchInput,
    ProviderSupportsRichSearch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    model: String,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    tools: Option<Vec<Tool>>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    enabled_providers: Option<Vec<String>>,
    #[serde(default)]
    kwargs: Option<SearchHookKwargs>,
    #[serde(default)]
    search_tools: Option<Vec<SearchToolConfig>>,
    #[serde(default)]
    search_tool_name: Option<String>,
    #[serde(default)]
    native_providers: Vec<String>,
    #[serde(default)]
    rich_providers: Vec<String>,
    #[serde(default)]
    search_response: Option<SearchResponse>,
    #[serde(default)]
    search_error: Option<ScriptedError>,
    #[serde(default)]
    deny_search_tool: bool,
    #[serde(default)]
    query: String,
    #[serde(default)]
    tool_inputs: Vec<Box<RawValue>>,
    #[serde(default)]
    search_providers: Vec<Option<String>>,
    // the Python helper patches the search-tool auth check when caller is set; the Rust cases carry that as kwargs and deny_search_tool
    #[serde(default, rename = "caller")]
    _caller: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptedError {
    kind: ErrorKind,
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ErrorKind {
    RateLimit,
    ContextWindow,
    Unavailable,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation {
            Operation::ShortCircuit | Operation::ExecuteSearch => operations::run_search(input),
            Operation::RichSearchInput => operations::rich_search_inputs(input),
            Operation::ProviderSupportsRichSearch => operations::provider_supports(input),
        }
    }
}

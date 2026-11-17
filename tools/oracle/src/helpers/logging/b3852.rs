use std::collections::BTreeMap;

use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::exporters::ExporterInput;

mod exporters;
mod fixtures;
mod harness;
mod scenarios;
mod spans;
mod tests;

pub(crate) struct Op;

// the one entry point the failure and success callbacks share, plus the tenant cache calls that bypass the logger
#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Function {
    PreCall,
    McpToolCall,
    MetricError,
    CloseDedup,
    CacheEviction,
    ProjectHeaders,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    callback_name: Option<String>,
    anchored: bool,
    exporters: Vec<ExporterInput>,
    credentials: Option<BTreeMap<String, String>>,
    auth_metadata: Option<BTreeMap<String, String>>,
    spaces: Vec<String>,
    project_name: Option<String>,
    filter: OtelModelValue,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::PreCall => scenarios::pre_call(input),
            Function::McpToolCall => scenarios::mcp_tool_call(input),
            Function::MetricError => scenarios::metric_error(input),
            Function::CloseDedup => scenarios::close_dedup(input),
            Function::CacheEviction => scenarios::cache_eviction(input),
            Function::ProjectHeaders => scenarios::project_headers(input),
        }
    }
}

mod session;
mod tests;

use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_mcp::tools::pagination::{Timeouts, ToolFormat, load_mcp_tools};
use betterlite_mcp::tools::schemas::{
    McpTool, mcp_tool_to_anthropic, mcp_tool_to_openai, mcp_tool_to_responses_api,
};
use betterlite_mcp::tools::tool_calls::{OpenAiToolCall, call_openai_tool};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "mcp_tool_schema")]
    Schema { tool: McpTool },
    #[serde(rename = "call")]
    Call { call: OpenAiToolCall },
    #[serde(rename = "pagination")]
    Pagination {
        #[serde(default)]
        pages: Vec<session::Frame>,
        #[serde(default)]
        generated_pages: usize,
        client_ms: u64,
        listing_ms: u64,
        deadline_ms: Option<u64>,
        #[serde(default)]
        format: ToolFormat,
    },
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(failure)?;
        runtime.block_on(run(input))
    }
}

async fn run(input: &Input) -> HelperResult {
    match input {
        Input::Schema { tool } => Ok(serde_json::json!({
            "openai": mcp_tool_to_openai(tool),
            "responses": mcp_tool_to_responses_api(tool),
            "anthropic": mcp_tool_to_anthropic(tool).map_err(failure)?,
        })),
        Input::Call { call } => invoke_call(call).await,
        Input::Pagination {
            pages,
            generated_pages,
            client_ms,
            listing_ms,
            deadline_ms,
            format,
        } => {
            let mut session = session::Fixture::from_pages(pages.clone(), *generated_pages);
            let warnings = session::Warnings::default();
            let count = std::sync::Arc::clone(&warnings.0);
            let subscriber = tracing_subscriber::layer::SubscriberExt::with(
                tracing_subscriber::registry(),
                warnings,
            );
            let _guard = tracing::subscriber::set_default(subscriber);
            let timeouts = Timeouts {
                client: Duration::from_millis(*client_ms),
                listing: Duration::from_millis(*listing_ms),
                listing_deadline: deadline_ms.map(Duration::from_millis),
            };
            let tools = load_mcp_tools(&mut session, *format, timeouts)
                .await
                .map_err(failure)?;
            Ok(
                serde_json::json!({"tools":tools,"cursors":session.cursors,"warnings":count.load(std::sync::atomic::Ordering::Relaxed)}),
            )
        }
    }
}

fn failure<Source>(error: Source) -> Box<HelperError>
where
    Source: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

async fn invoke_call(call: &OpenAiToolCall) -> HelperResult {
    let mut session = session::Fixture::default();
    let timeouts = Timeouts {
        client: Duration::from_secs(60),
        listing: Duration::from_secs(30),
        listing_deadline: None,
    };
    serde_json::to_value(
        call_openai_tool(&mut session, call, timeouts)
            .await
            .map_err(failure)?,
    )
    .map_err(failure)
}

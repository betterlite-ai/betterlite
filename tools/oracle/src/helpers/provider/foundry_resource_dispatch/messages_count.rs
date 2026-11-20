use std::time::Duration;

use betterlite_foundry::anthropic_count::handler::FoundryCountHandler;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::foundry_resource_dispatch::messages_count::{Call, Dispatch, Outcome};
use betterlite_provider::foundry_resource_dispatch::{
    FoundryResourceOperation, FoundryResourceResult, dispatch,
};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Runtime;

use super::support::{NoTransport, OTHER, failure, runtime};

#[derive(Debug, Deserialize)]
pub(super) struct Input {
    api_base: Option<String>,
}

// B564's Messages endpoint and B565's count endpoint are separate native paths from one base
pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let handler = FoundryCountHandler {
        transport: NoTransport,
        request_timeout: Duration::from_secs(5),
        beta_version: String::from("2024-06-01"),
        configured_key: None,
    };
    let messages = dispatch_call(
        &handler,
        Call::MessagesUrl {
            api_base: input.api_base.as_deref(),
            environment_api_base: None,
        },
        &runtime,
    )?;
    let count = dispatch_call(
        &handler,
        Call::CountUrl {
            api_base: input.api_base.as_deref().unwrap_or_default(),
        },
        &runtime,
    )?;
    match (messages, count) {
        (Outcome::MessagesUrl(messages), Outcome::CountUrl(count)) => {
            Ok(json!({"messages": messages, "count": count}))
        }
        _ => Err(failure("Variant", OTHER)),
    }
}

fn dispatch_call(
    handler: &FoundryCountHandler<NoTransport>,
    call: Call<'_>,
    runtime: &Runtime,
) -> Result<Outcome, Box<HelperError>> {
    match runtime.block_on(dispatch(FoundryResourceOperation::MessagesCount(
        Dispatch { handler, call }.run(),
    ))) {
        Ok(FoundryResourceResult::MessagesCount(outcome)) => Ok(outcome),
        Ok(_) => Err(failure("Variant", OTHER)),
        Err(error) => Err(failure("LeafFailure", error)),
    }
}

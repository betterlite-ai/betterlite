use std::convert::Infallible;

use betterlite_guardrails::cisco_conversation::Fields;
use betterlite_guardrails::cisco_inspection::hooks::PostCallArgs;
use betterlite_guardrails::cisco_stream_scan::scan_stream;
use betterlite_oracle_helper_api::helper::HelperResult;
use futures_util::stream::iter;
use serde_json::{Value, json};

use super::StreamArgs;
use super::projection;
use super::support::{Harness, applied_guardrails};

pub(super) fn run(args: &StreamArgs) -> HelperResult {
    let harness = Harness::start(&args.answer)?;
    let guardrail = harness.guardrail(&args.settings)?;
    let mut request_data: Fields =
        serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let chunks = args.chunks.iter().cloned().map(Ok::<_, Infallible>);
    let items = harness
        .block_on(scan_stream(
            &guardrail,
            &mut request_data,
            iter(chunks),
            PostCallArgs {
                should_run: args.should_run,
                key_user_id: args.key_user_id.as_deref(),
            },
        ))
        .unwrap_or_default();
    let (path, inspected) = harness.recorded()?;
    Ok(json!({
        "items": items.iter().map(projection::stream_item).collect::<Vec<Value>>(),
        "path": path,
        "request": inspected,
        "applied_guardrails": applied_guardrails(&request_data),
    }))
}

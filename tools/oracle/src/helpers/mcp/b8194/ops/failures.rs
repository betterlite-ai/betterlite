use betterlite_server::mcp::debug::preview::describe_upstream_http_failure;
use betterlite_server::mcp::debug::preview::upstream::{UpstreamFailure, UpstreamResponse};
use serde_json::json;

use crate::helpers::HelperResult;

use super::super::input::FailureInput;
use super::{mapped, redactor, response_list};

pub(super) fn describe_failure(input: &FailureInput) -> HelperResult {
    let redactor = redactor()?;
    let responses = response_list(&input.responses)?;
    let descriptions = mapped(&input.chains, |chain| {
        let root = failure_chain(chain, &responses);
        describe_upstream_http_failure(&root, &redactor)
    });
    Ok(json!({ "descriptions": descriptions }))
}

struct FailureNode {
    response: Option<UpstreamResponse>,
    cause: Option<Box<Self>>,
}

impl UpstreamFailure for FailureNode {
    fn response(&self) -> Option<&UpstreamResponse> {
        self.response.as_ref()
    }

    fn cause(&self) -> Option<&dyn UpstreamFailure> {
        let node = self.cause.as_deref()?;
        Some(node)
    }
}

fn failure_chain(entries: &[Option<usize>], responses: &[UpstreamResponse]) -> FailureNode {
    let mut cause: Option<Box<FailureNode>> = None;
    for entry in entries.iter().rev() {
        let response = entry.and_then(|index| responses.get(index)).cloned();
        cause = Some(Box::new(FailureNode { response, cause }));
    }
    cause.map_or_else(
        || FailureNode {
            response: None,
            cause: None,
        },
        |node| *node,
    )
}

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UndeliverableRewriteReason {
    #[error("{missing} of the stream's {total} tool call items carry no call_id")]
    MissingCallId { missing: usize, total: usize },
    #[error("the stream's tool call items repeat a call_id")]
    RepeatedCallId,
    #[error(
        "the guardrail returned {returned} tool calls for the stream's {expected} tool call items"
    )]
    CallCountMismatch { returned: usize, expected: usize },
    #[error("a tool call argument event names an item_id that no output_item event introduced")]
    UnresolvedArgumentEvent,
    #[error("no stream event carries the rewritten call_id {}", missing.join(", "))]
    MissingEventCallId { missing: Vec<String> },
}

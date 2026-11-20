use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse as _, Response};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::debug_routes::DebugRoutesState;
use crate::tracing_routes::router::unauthenticated;

mod tests;

const MINIMUM_START_TIME: u64 = 1_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedSpan {
    pub name: String,
    pub parent_trace_id: Option<u128>,
    pub start_time_nanos: u64,
}

pub trait FinishedSpanSource: Send + Sync {
    fn finished_spans(&self) -> Vec<RecordedSpan>;
}

#[derive(Serialize)]
struct OtelSpansBody {
    otel_spans: Vec<String>,
    spans_grouped_by_parent: BTreeMap<String, Vec<String>>,
    most_recent_parent: Option<u128>,
}

pub(super) async fn handler(
    State(state): State<DebugRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let spans = state
        .spans
        .as_ref()
        .map_or_else(Vec::new, |source| source.finished_spans());
    Json(group_spans(&spans)).into_response()
}

fn group_spans(spans: &[RecordedSpan]) -> OtelSpansBody {
    let mut spans_grouped_by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut most_recent_parent = None;
    let mut most_recent_start_time = MINIMUM_START_TIME;
    for span in spans {
        let Some(parent_trace_id) = span.parent_trace_id else {
            continue;
        };
        spans_grouped_by_parent
            .entry(parent_trace_id.to_string())
            .or_default()
            .push(span.name.clone());
        if span.start_time_nanos > most_recent_start_time {
            most_recent_parent = Some(parent_trace_id);
            most_recent_start_time = span.start_time_nanos;
        }
    }
    OtelSpansBody {
        otel_spans: spans.iter().map(|span| span.name.clone()).collect(),
        spans_grouped_by_parent,
        most_recent_parent,
    }
}

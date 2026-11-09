use chrono::{DateTime, Utc};
use uuid::Uuid;

// The run fields _ensure_required_ids fills, optional so a run that arrives without one is visible.
#[derive(Clone, Debug, Default)]
pub struct RunIds {
    pub id: Option<String>,
    pub parent_run_id: Option<String>,
    pub trace_id: Option<String>,
    pub dotted_order: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredIds {
    pub id: String,
    pub trace_id: String,
    pub dotted_order: String,
}

// str(uuid.uuid4()) for a run that arrives without an id, injected so a test fixes the value.
pub trait IdSource: Send + Sync {
    fn next_id(&self) -> String;
}

#[derive(Debug, Default)]
pub struct SystemIds;

impl IdSource for SystemIds {
    fn next_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

// LangsmithLogger._ensure_required_ids: a root run's trace id must be the id in its dotted order, or LangSmith drops the batch.
#[must_use]
pub fn ensure_required_ids(
    ids: &RunIds,
    run_id: Option<&str>,
    now: DateTime<Utc>,
    source: &dyn IdSource,
) -> RequiredIds {
    let resolved_id = run_id.map_or_else(|| source.next_id(), str::to_owned);
    let posts_as_root = ids.parent_run_id.is_none() && ids.dotted_order.is_none();
    RequiredIds {
        id: ids.id.clone().unwrap_or_else(|| resolved_id.clone()),
        trace_id: if posts_as_root {
            resolved_id.clone()
        } else {
            ids.trace_id.clone().unwrap_or_else(|| resolved_id.clone())
        },
        // strftime("%Y%m%dT%H%M%S%fZ") + run id, with no separator between the timestamp and the id.
        dotted_order: ids
            .dotted_order
            .clone()
            .unwrap_or_else(|| format!("{}Z{}", now.format("%Y%m%dT%H%M%S%6f"), resolved_id)),
    }
}
